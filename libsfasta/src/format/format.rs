use std::io::{Read, Seek, SeekFrom};
use std::sync::RwLock;
use std::thread;
use std::time::{Duration, Instant};

use bincode::Options;
use bumpalo::Bump;
use rayon::prelude::*;
use serde_bytes::ByteBuf;

use super::Directory;
use crate::index::*;
use crate::index_directory::IndexDirectory;
use crate::metadata::Metadata;
use crate::parameters::Parameters;
use crate::sequence_block::*;
use crate::types::*;
use crate::*;

// TODO: Move these into parameters
pub const IDX_CHUNK_SIZE: usize = 128 * 1024;
pub const SEQLOCS_CHUNK_SIZE: usize = 128 * 1024;

pub struct Sfasta {
    pub version: u64, // I'm going to regret this, but 18,446,744,073,709,551,615 versions should be enough for anybody.
    pub directory: Directory,
    pub parameters: Parameters,
    pub metadata: Metadata,
    pub index_directory: IndexDirectory,
    pub index: Option<Index64>,
    pub index_plan: Option<StoredIndexPlan>,
    buf: Option<RwLock<Box<dyn ReadAndSeek>>>, // TODO: Needs to be behind RwLock to support better multi-threading...
    pub block_locs: Option<Vec<u64>>,
    pub id_blocks_locs: Option<Vec<u64>>,
    pub seqlocs_blocks_locs: Option<Vec<u64>>,
}

impl Default for Sfasta {
    fn default() -> Self {
        Sfasta {
            version: 1,
            directory: Directory::default(),
            parameters: Parameters::default(),
            metadata: Metadata::default(),
            index_directory: IndexDirectory::default().with_blocks().with_ids(),
            index: None,
            buf: None,
            block_locs: None,
            id_blocks_locs: None,
            seqlocs_blocks_locs: None,
        }
    }
}

impl Sfasta {
    pub fn with_sequences(self) -> Self {
        // TODO: Should we really have SFASTA without sequences?!
        // self.directory = self.directory.with_sequences();
        self
    }
    sfasta.directory -> Self {
        self.directory = self.directory.with_scores();
        self
    }

    pub fn with_masking(mut self) -> Self {
        self.directory = self.directory.with_masking();
        self
    }

    pub fn block_size(mut self, block_size: u32) -> Self {
        self.parameters.block_size = block_size;
        self
    }

    // TODO: Does nothing right now...
    pub fn compression_type(mut self, compression: CompressionType) -> Self {
        self.parameters.compression_type = compression;
        self
    }

    pub fn decompress_all_ids(&mut self) {
        assert!(
            self.buf.is_some(),
            "Sfasta buffer not yet present -- Are you creating a file?"
        );
        let len = self.index.as_ref().unwrap().len();

        let blocks = (len as f64 / IDX_CHUNK_SIZE as f64).ceil() as usize;

        /* self.buf
        .as_deref_mut()
        .unwrap()
        .seek(SeekFrom::Start(self.directory.ids_loc))
        .expect("Unable to work with seek API"); */

        let mut buf = self.buf.as_ref().unwrap().write().unwrap();
        buf.seek(SeekFrom::Start(self.directory.ids_loc))
            .expect("Unable to work with seek API");

        let mut ids: Vec<String> = Vec::with_capacity(len as usize);

        // Multi-threaded
        let mut compressed_blocks: Vec<ByteBuf> = Vec::with_capacity(blocks);
        for _i in 0..blocks {
            compressed_blocks.push(bincode::deserialize_from(&mut *buf).unwrap());
            // println!("Processed block {}", i);
        }

        let output: Vec<Vec<String>> = compressed_blocks
            .par_iter()
            .map(|x| {
                let mut decoder = zstd::stream::Decoder::new(&x[..]).unwrap();

                let mut decompressed = Vec::with_capacity(8 * 1024 * 1024);

                match decoder.read_to_end(&mut decompressed) {
                    Ok(x) => x,
                    Err(y) => panic!("Unable to decompress block: {:#?}", y),
                };

                bincode::deserialize_from(&decompressed[..]).unwrap()
            })
            .collect();

        output.into_iter().for_each(|x| ids.extend(x));

        // Single-threaded
        /*for _ in 0..blocks {
            let compressed: ByteBuf;
            compressed = bincode::deserialize_from(&mut *buf).unwrap();
            // let mut decompressed = lz4_flex::frame::FrameDecoder::new(&compressed[..]);
            let mut decoder = zstd::stream::Decoder::new(&compressed[..]).unwrap();

            let mut decompressed = Vec::with_capacity(8 * 1024 * 1024);

            match decoder.read_to_end(&mut decompressed) {
                Ok(x) => x,
                Err(y) => panic!("Unable to decompress block: {:#?}", y),
            };

            let chunk_ids: Vec<String>;
            chunk_ids = bincode::deserialize_from(&decompressed[..]).unwrap();
            ids.extend(chunk_ids);
        } */

        assert!(ids.len() == self.index.as_ref().unwrap().locs.len());

        self.index.as_mut().unwrap().set_ids(ids);
    }

    pub fn find(&self, x: &str) -> Result<Option<Vec<(String, usize, u32, Vec<Loc>)>>, &str> {
        let possibilities = self.index.as_ref().unwrap().find(&x);

        if possibilities.is_some() {
            let possibilities = possibilities.unwrap();

            let mut matches: Vec<(String, usize, u32)> = Vec::with_capacity(possibilities.len());
            let locs = &self.index.as_ref().unwrap().locs;

            if self.index.as_ref().unwrap().ids.is_some() {
                // Index is already decompressed, just search it appropriately...

                let idx_ref = self.index.as_ref().unwrap().ids.as_ref().unwrap();

                for loc in possibilities {
                    if idx_ref[loc as usize] == x {
                        matches.push((
                            idx_ref[loc as usize].clone(),
                            loc as usize,
                            locs[loc as usize],
                        ));
                    }
                }
            } else {
                // let mut bump = Bump::new();
                for loc in possibilities {
                    let block = loc as usize / IDX_CHUNK_SIZE;

                    let mut buf = self.buf.as_ref().unwrap().write().unwrap();
                    /*self.buf
                    .as_deref_mut()
                    .unwrap()
                    .seek(SeekFrom::Start(self.directory.ids_loc))
                    .expect("Unable to work with seek API"); */

                    buf.seek(SeekFrom::Start(
                        self.id_blocks_locs.as_ref().unwrap()[block],
                    ))
                    .expect("Unable to work with SEEK API");

                    let mut decompressed: Vec<u8> = Vec::with_capacity(2 * 1024 * 1024);

                    //let compressed: &mut ByteBuf =
                    //    bump.alloc(bincode::deserialize_from(&mut *buf).unwrap());
                    let compressed: ByteBuf = bincode::deserialize_from(&mut *buf).unwrap();
                    let mut decoder = zstd::stream::Decoder::new(&compressed[..]).unwrap();

                    match decoder.read_to_end(&mut decompressed) {
                        Ok(x) => x,
                        Err(y) => panic!("Unable to decompress block: {:#?}", y),
                    };

                    // let mut decompressed =
                    //     bump.alloc(lz4_flex::frame::FrameDecoder::new(&compressed[..]));
                    //let ids: &mut Vec<String> =
                    //bump.alloc(bincode::deserialize_from(&decompressed[..]).unwrap());
                    let ids: Vec<String> = bincode::deserialize_from(&decompressed[..]).unwrap();

                    if ids[loc as usize % IDX_CHUNK_SIZE] == x {
                        matches.push((
                            ids[loc as usize % IDX_CHUNK_SIZE].clone(),
                            loc as usize,
                            locs[loc as usize % IDX_CHUNK_SIZE],
                        ));
                    }
                    // bump.reset();
                }
            }

            let return_val;
            let mut matches_with_loc: Vec<(String, usize, u32, Vec<Loc>)> = Vec::new();
            if matches.len() > 0 {
                let mut buf = self.buf.as_ref().unwrap().write().unwrap();
                for (id, idxloc, loc) in matches {
                    /*self.buf
                    .as_deref_mut()
                    .unwrap()
                    .seek(SeekFrom::Start(self.directory.seqlocs_loc))
                    .expect("Unable to work with seek API"); */
                    buf.seek(SeekFrom::Start(self.directory.seqlocs_loc))
                        .expect("Unable to work with seek API");
                    let block = loc as usize / SEQLOCS_CHUNK_SIZE;

                    buf.seek(SeekFrom::Start(
                        self.seqlocs_blocks_locs.as_ref().unwrap()[block],
                    ))
                    .expect("Unable to work with seek API");
                    let compressed: ByteBuf = bincode::deserialize_from(&mut *buf).unwrap();

                    let mut decoder = zstd::stream::Decoder::new(&compressed[..]).unwrap();
                    let mut decompressed = Vec::with_capacity(2 * 1024 * 1024);

                    match decoder.read_to_end(&mut decompressed) {
                        Ok(x) => x,
                        Err(y) => panic!("Unable to decompress block: {:#?}", y),
                    };

                    //let mut decompressed = lz4_flex::frame::FrameDecoder::new(&compressed[..]);
                    let seqlocs: Vec<Vec<Loc>> =
                        bincode::deserialize_from(&decompressed[..]).unwrap();

                    let thisloc = seqlocs[loc as usize % SEQLOCS_CHUNK_SIZE].clone();
                    matches_with_loc.push((id, idxloc, loc, thisloc));
                }
                return_val = Some(matches_with_loc);
            } else {
                return_val = None;
            }

            Ok(return_val)
        } else {
            Ok(None)
        }
    }

    pub fn get_sequence(&self, locs: &Vec<Loc>) -> Result<Vec<u8>, &'static str> {
        let mut seq: Vec<u8> = Vec::with_capacity(2 * 1024 * 1024); // TODO: We can calculate this

        if locs.len() == 0 {
            return Err("No locations passed, Vec<Loc> is empty");
        }

        // Basic sanity checks
        for (i, _) in locs {
            if *i as usize >= self.block_locs.as_ref().unwrap().len() {
                return Err("Requested block number is larger than the total number of blocks");
            }
        }

        // TODO: Probably no benefit here......
        // let mut bump = Bump::new();

        for loc in locs {
            let byte_loc = self.block_locs.as_ref().unwrap()[loc.0 as usize];
            let mut buf = self.buf.as_ref().unwrap().write().unwrap();
            /*self.buf
            .as_deref_mut()
            .unwrap()
            .seek(SeekFrom::Start(byte_loc))
            .expect("Unable to work with seek API"); */
            buf.seek(SeekFrom::Start(byte_loc))
                .expect("Unable to work with seek API");

            let sbc: SequenceBlockCompressed = bincode::deserialize_from(&mut *buf)
                .expect("Unable to parse SequenceBlockCompressed");

            drop(buf); // Open it up for other threads...
                       // let sb = bump.alloc(sbc.decompress(self.parameters.compression_type));
            let sb = sbc.decompress(self.parameters.compression_type);

            seq.extend_from_slice(&sb.seq[loc.1 .0 as usize..loc.1 .1 as usize]);
            // bump.reset();
        }

        Ok(seq)
    }

    pub fn index_len(&self) -> usize {
        self.index.as_ref().unwrap().index_len as usize
    }
}

pub struct SfastaParser {
    pub sfasta: Sfasta,
}

impl SfastaParser {
    // TODO: Can probably multithread parts of this...
    pub fn open_from_buffer<R>(mut in_buf: R) -> Sfasta
    where
        R: 'static + Read + Seek + Send,
    {
        let bincode = bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .allow_trailing_bytes();

        let mut sfasta_marker: [u8; 6] = [0; 6];
        in_buf
            .read_exact(&mut sfasta_marker)
            .expect("Unable to read SFASTA Marker");
        assert!(sfasta_marker == "sfasta".as_bytes());

        let mut sfasta = Sfasta::default();

        sfasta.version = match bincode::deserialize_from(&mut in_buf) {
            Ok(x) => x,
            Err(y) => panic!("Error reading SFASTA directory: {}", y),
        };

        assert!(sfasta.version <= 1); // 1 is the maximum version supported at this stage...

        // TODO: In the future, with different versions, we will need to do different things
        // when we inevitabily introduce incompatabilities...

        sfasta.directory = match bincode::deserialize_from(&mut in_buf) {
            Ok(x) => x,
            Err(y) => panic!("Error reading SFASTA directory: {}", y),
        };

        sfasta.parameters = match bincode::deserialize_from(&mut in_buf) {
            Ok(x) => x,
            Err(y) => panic!("Error reading SFASTA parameters: {}", y),
        };

        sfasta.metadata = match bincode::deserialize_from(&mut in_buf) {
            Ok(x) => x,
            Err(y) => panic!("Error reading SFASTA metadata: {}", y),
        };

        // Next are the sequence blocks, which aren't important right now...
        // The index is much more important to us...

        // TODO: Fix for when no index
        in_buf
            .seek(SeekFrom::Start(sfasta.directory.index_loc.unwrap()))
            .expect("Unable to work with seek API");

        // TODO: Parse index plan here...

        let plan: StoredIndexPlan = bincode
            .deserialize_from(&mut in_buf)
            .expect("Unable to get Hash Type of Index");

        sfasta.index_plan = Some(plan);
        sfasta.index = Index64::from_parts(hashes, locs, hash);

        /*

        let hashes_compressed: ByteBuf = bincode
            .deserialize_from(&mut in_buf)
            .expect("Unable to parse index");

        let bitpacked: Vec<Bitpacked> = bincode
            .deserialize_from(&mut in_buf)
            .expect("Unable to get Bitpacked Index");

        // Parse and decompress in the index in another thread
        let index_handle = thread::spawn(move || {
            // let mut decompressor = lz4_flex::frame::FrameDecoder::new(&index_compressed[..]);

            let mut decompressor = zstd::stream::Decoder::new(&hashes_compressed[..]).unwrap();
            let mut index_bincoded = Vec::with_capacity(64 * 1024 * 1024);

            decompressor
                .read_to_end(&mut index_bincoded)
                .expect("Unable to parse index");

            let hashes: Vec<u64> = bincode
                .deserialize_from(&index_bincoded[..])
                .expect("Unable to parse index");

            let locs: Vec<u32> = unbitpack_u32(bitpacked)
                .into_iter()
                .map(|x| x + min_size)
                .collect();

            Index64::from_parts(hashes, locs, hash_type)
        });

        */

        in_buf
            .seek(SeekFrom::Start(sfasta.directory.block_index_loc))
            .expect("Unable to work with seek API");

        let block_locs_compressed: ByteBuf =
            bincode::deserialize_from(&mut in_buf).expect("Unable to parse block locs index");

        let block_locs_handle = thread::spawn(move || {
            // let mut decompressor = lz4_flex::frame::FrameDecoder::new(&block_locs_compressed[..]);
            let mut decompressor = zstd::stream::Decoder::new(&block_locs_compressed[..]).unwrap();
            let mut block_locs_bincoded: ByteBuf = ByteBuf::new(); //Vec::with_capacity(2 * 1024 * 1024);

            decompressor
                .read_to_end(&mut block_locs_bincoded)
                .expect("Unable to parse index");

            let block_locs: Vec<u64> = bincode
                .deserialize_from(&block_locs_bincoded[..])
                .expect("Unable to parse index");

            block_locs
        });

        let mut id_blocks_index_handle = None;

        if sfasta.directory.id_blocks_index_loc.is_some() {
            in_buf
                .seek(SeekFrom::Start(
                    sfasta.directory.id_blocks_index_loc.unwrap(),
                ))
                .expect("Unable to work with seek API");

            let id_blocks_locs_compressed: Vec<Bitpacked> =
                bincode::deserialize_from(&mut in_buf).expect("Unable to parse block locs index");

            let ids_loc = sfasta.directory.ids_loc;
            id_blocks_index_handle = Some(thread::spawn(move || {
                //let mut decompressor =
                //    lz4_flex::frame::FrameDecoder::new(&id_blocks_locs_compressed[..]);
                /*let mut decompressor =
                    zstd::stream::Decoder::new(&id_blocks_locs_compressed[..]).unwrap();

                let mut id_blocks_locs_bincoded: Vec<u8> = Vec::with_capacity(2 * 1024 * 1024);

                decompressor
                    .read_to_end(&mut id_blocks_locs_bincoded)
                    .expect("Unable to parse index");

                let id_blocks_locs: Vec<u64> = bincode
                    .deserialize_from(&id_blocks_locs_bincoded[..])
                    .expect("Unable to parse index"); */

                let id_blocks_locs: Vec<u32> = unbitpack_u32(id_blocks_locs_compressed);

                let id_blocks_locs: Vec<u64> = id_blocks_locs
                    .into_iter()
                    .map(|x| x as u64 + ids_loc)
                    .collect();

                id_blocks_locs
            }));
        }

        let mut seqloc_blocks_handle = None;

        if sfasta.directory.seqloc_blocks_index_loc.is_some() {
            in_buf
                .seek(SeekFrom::Start(
                    sfasta.directory.seqloc_blocks_index_loc.unwrap(),
                ))
                .expect("Unable to work with seek API");

            let seqlocs_blocks_locs_compressed: ByteBuf =
                bincode::deserialize_from(&mut in_buf).expect("Unable to parse block locs index");

            let seqlocs_loc = sfasta.directory.seqlocs_loc;
            seqloc_blocks_handle = Some(thread::spawn(move || {
                let mut decompressor =
                    zstd::stream::Decoder::new(&seqlocs_blocks_locs_compressed[..]).unwrap();
                let mut seqlocs_blocks_locs_bincoded: Vec<u8> = Vec::with_capacity(2 * 1024 * 1024);

                decompressor
                    .read_to_end(&mut seqlocs_blocks_locs_bincoded)
                    .expect("Unable to parse index");

                let seqlocs_blocks_locs: Vec<u64> = bincode
                    .deserialize_from(&seqlocs_blocks_locs_bincoded[..])
                    .expect("Unable to parse index");

                let seqlocs_blocks_locs: Vec<u64> = seqlocs_blocks_locs
                    .into_iter()
                    .map(|x| x + seqlocs_loc)
                    .collect();

                seqlocs_blocks_locs
            }));
        }

        // TODO: Disabled for testing...
        // If there are few enough IDs, let's decompress it and store it in the index...
        // if parser.sfasta.index.as_ref().unwrap().len() <= 8192 * 2 {
        //    parser.decompress_all_ids();
        // }

        // sfasta.index = Some(index_handle.join().unwrap());
        sfasta.block_locs = Some(block_locs_handle.join().unwrap());
        sfasta.buf = Some(RwLock::new(Box::new(in_buf)));

        if id_blocks_index_handle.is_some() {
            sfasta.id_blocks_locs = Some(id_blocks_index_handle.unwrap().join().unwrap());
        }

        if seqloc_blocks_handle.is_some() {
            sfasta.seqlocs_blocks_locs = Some(seqloc_blocks_handle.unwrap().join().unwrap());
        }

        sfasta
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversion::Converter;
    use std::fs::File;
    use std::io::BufReader;
    use std::io::Cursor;

    #[test]
    pub fn test_sfasta_find_and_retrieve_sequence() {
        let mut out_buf = Cursor::new(Vec::new());

        let in_buf = BufReader::new(
            File::open("test_data/test_convert.fasta").expect("Unable to open testing file"),
        );

        let converter = Converter::default()
            .with_threads(6)
            .with_block_size(8 * 1024)
            .with_index();

        converter.convert_fasta(in_buf, &mut out_buf);

        match out_buf.seek(SeekFrom::Start(0)) {
            Err(x) => panic!("Unable to seek to start of file, {:#?}", x),
            Ok(_) => (),
        };

        let mut sfasta = SfastaParser::open_from_buffer(out_buf);
        assert!(sfasta.index_len() == 3001);

        let output = sfasta.find("does-not-exist");
        assert!(output == Ok(None));

        let output = &sfasta.find("needle").unwrap().unwrap()[0];
        assert!(output.0 == "needle");

        let output = &sfasta.find("needle_last").unwrap().unwrap()[0];
        assert!(output.0 == "needle_last");

        let sequence = sfasta.get_sequence(&output.3).unwrap();
        let sequence = std::str::from_utf8(&sequence).unwrap();
        assert!("ACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATA" == sequence);
    }

    #[test]
    pub fn test_parse_multiple_blocks() {
        let mut out_buf = Cursor::new(Vec::new());

        let in_buf = BufReader::new(
            File::open("test_data/test_sequence_conversion.fasta")
                .expect("Unable to open testing file"),
        );

        let converter = Converter::default()
            .with_threads(6)
            .with_block_size(512)
            .with_index();

        converter.convert_fasta(in_buf, &mut out_buf);

        match out_buf.seek(SeekFrom::Start(0)) {
            Err(x) => panic!("Unable to seek to start of file, {:#?}", x),
            Ok(_) => (),
        };

        let mut sfasta = SfastaParser::open_from_buffer(out_buf);
        assert!(sfasta.index_len() == 10);

        let output = &sfasta.find("test3").unwrap().unwrap()[0];
        assert!(output.0 == "test3");

        let sequence = sfasta.get_sequence(&output.3).unwrap();
        let sequence = std::str::from_utf8(&sequence).unwrap();

        println!("{:#?}", sequence.len());

        assert!(sequence.len() == 48598);
        assert!(&sequence[0..100] == "ATGCGATCCGCCCTTTCATGACTCGGGTCATCCAGCTCAATAACACAGACTATTTTATTGTTCTTCTTTGAAACCAGAACATAATCCATTGCCATGCCAT");
        assert!(&sequence[48000..48100] == "AACCGGCAGGTTGAATACCAGTATGACTGTTGGTTATTACTGTTGAAATTCTCATGCTTACCACCGCGGAATAACACTGGCGGTATCATGACCTGCCGGT");
    }

    #[test]
    pub fn test_find_does_not_trigger_infinite_loops() {
        let mut out_buf = Cursor::new(Vec::new());

        let in_buf = BufReader::new(
            File::open("test_data/test_sequence_conversion.fasta")
                .expect("Unable to open testing file"),
        );

        let converter = Converter::default()
            .with_threads(6)
            .with_block_size(512)
            .with_threads(8);

        converter.convert_fasta(in_buf, &mut out_buf);

        match out_buf.seek(SeekFrom::Start(0)) {
            Err(x) => panic!("Unable to seek to start of file, {:#?}", x),
            Ok(_) => (),
        };

        let mut sfasta = SfastaParser::open_from_buffer(out_buf);
        assert!(sfasta.index_len() == 10);

        let output = &sfasta.find("test3").unwrap().unwrap()[0];
        assert!(output.0 == "test3");
        &sfasta.find("test").unwrap().unwrap()[0];
        &sfasta.find("test2").unwrap().unwrap()[0];
        &sfasta.find("test3").unwrap().unwrap()[0];
        &sfasta.find("test4").unwrap().unwrap()[0];
        &sfasta.find("test5").unwrap().unwrap()[0];
    }
}
