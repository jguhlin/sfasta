use std::io::{Read, Seek, SeekFrom};
use std::sync::RwLock;

use rayon::prelude::*;

use crate::datatypes::*;
use crate::dual_level_index::*;
use crate::*;

// TODO: Move these into parameters
pub const IDX_CHUNK_SIZE: usize = 128 * 1024;

pub struct Sfasta {
    pub version: u64, // I'm going to regret this, but 18,446,744,073,709,551,615 versions should be enough for anybody.
    pub directory: Directory,
    pub parameters: Parameters,
    pub metadata: Metadata,
    pub index_directory: IndexDirectory,
    pub index: Option<DualIndex>,
    buf: Option<RwLock<Box<dyn ReadAndSeek>>>, // TODO: Needs to be behind RwLock to support better multi-threading...
    pub sequenceblocks: Option<SequenceBlocks>,
    pub seqlocs: Option<SeqLocs>,
    pub headers: Option<Headers>,
    pub ids: Option<Ids>,
    pub masking: Option<Masking>,
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
            sequenceblocks: None,
            seqlocs: None,
            headers: None,
            ids: None,
            masking: None,
        }
    }
}

impl Sfasta {
    pub fn with_sequences(self) -> Self {
        // TODO: Should we really have SFASTA without sequences?!
        // self.directory = self.directory.with_sequences();
        self
    }

    pub fn with_scores(mut self) -> Self {
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
        // let len = self.index.as_ref().unwrap().len();

        let bincode_config = bincode::config::standard().with_fixed_int_encoding();
        let len = 1000; // TODO: Ussing to make this compile...

        let blocks = (len as f64 / IDX_CHUNK_SIZE as f64).ceil() as usize;

        let mut buf = self.buf.as_ref().unwrap().write().unwrap();
        buf.seek(SeekFrom::Start(self.directory.ids_loc.unwrap().get()))
            .expect("Unable to work with seek API");

        let mut ids: Vec<String> = Vec::with_capacity(len as usize);

        // Multi-threaded
        let mut compressed_blocks: Vec<Vec<u8>> = Vec::with_capacity(blocks);
        for _i in 0..blocks {
            compressed_blocks
                .push(bincode::decode_from_std_read(&mut *buf, bincode_config).unwrap());
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

                let (r, _) = bincode::decode_from_slice(&decompressed, bincode_config).unwrap();
                r
            })
            .collect();

        output.into_iter().for_each(|x| ids.extend(x));

        // TODO: FIXME
        // self.index.as_mut().unwrap().set_ids(ids);
    }

    pub fn find(&mut self, x: &str) -> Result<Option<SeqLoc>, &str> {
        assert!(self.index.is_some(), "Sfasta index not present");

        let idx = self.index.as_mut().unwrap();
        let mut buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        let found = idx.find(&mut buf, x);
        let seqlocs = self.seqlocs.as_mut().unwrap();

        if found.is_none() {
            return Ok(None);
        }

        // TODO: Allow returning multiple if there are multiple matches...
        Ok(Some(seqlocs.get_seqloc(&mut buf, found.unwrap())))
    }

    pub fn get_header(&mut self, locs: &[Loc]) -> Result<String, &'static str> {
        let headers = self.headers.as_mut().unwrap();
        let mut buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        Ok(headers.get_header(&mut buf, locs))
    }

    pub fn get_id(&mut self, locs: &[Loc]) -> Result<String, &'static str> {
        let ids = self.ids.as_mut().unwrap();
        let mut buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        Ok(ids.get_id(&mut buf, locs))
    }

    pub fn get_sequence(&mut self, seqloc: &SeqLoc) -> Result<Vec<u8>, &'static str> {
        let mut seq: Vec<u8> = Vec::with_capacity(2 * 1024 * 1024); // TODO: We can calculate this

        seqloc
            .sequence
            .as_ref()
            .expect("No locations passed, Vec<Loc> is empty");

        let locs = seqloc.sequence.as_ref().unwrap();

        // Basic sanity checks
        let max_block = locs
            .iter()
            .map(|x| x.original_format(self.parameters.block_size))
            .map(|(x, _)| x)
            .max()
            .unwrap();
        assert!(
            self.sequenceblocks.as_ref().unwrap().block_locs.len() > max_block as usize,
            "Requested block is larger than the total number of blocks."
        );

        let mut buf = self.buf.as_ref().unwrap().write().unwrap();

        for (block, (start, end)) in locs
            .iter()
            .map(|x| x.original_format(self.parameters.block_size))
        {
            let seqblock = self
                .sequenceblocks
                .as_mut()
                .unwrap()
                .get_block(&mut *buf, block);
            seq.extend_from_slice(&seqblock[start as usize..end as usize]);
        }

        if seqloc.masking.is_some() && self.masking.is_some() {
            let masked_loc = seqloc.masking.as_ref().unwrap();
            let masking = self.masking.as_mut().unwrap();
            masking.mask_sequence(&mut *buf, *masked_loc, &mut seq);
        }

        Ok(seq)
    }

    pub fn len(&self) -> usize {
        self.seqlocs.as_ref().unwrap().len()
    }

    pub fn get_seqloc(&mut self, i: usize) -> SeqLoc {
        assert!(i < self.len(), "Index out of bounds");
        assert!(i < std::u32::MAX as usize, "Index out of bounds");

        let mut buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        self.seqlocs
            .as_mut()
            .unwrap()
            .get_seqloc(&mut buf, i as u32)
    }

    pub fn index_len(&mut self) -> usize {
        if self.index.is_none() {
            return 0;
        }

        self.index.as_mut().unwrap().len()
    }
}

pub struct SfastaParser {
    pub sfasta: Sfasta,
}

impl SfastaParser {
    pub fn open(path: &str) -> Result<Sfasta, &'static str> {
        let in_buf = std::fs::File::open(path).expect("Unable to open file");
        let mut sfasta = SfastaParser::open_from_buffer(
            std::io::BufReader::with_capacity(8 * 1024, in_buf),
            false,
        );

        Ok(sfasta)
    }

    // TODO: Can probably multithread parts of this...
    // Prefetch should probably be another name...
    pub fn open_from_buffer<R>(mut in_buf: R, prefetch: bool) -> Sfasta
    where
        R: 'static + Read + Seek + Send,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        let mut sfasta_marker: [u8; 6] = [0; 6];
        in_buf
            .read_exact(&mut sfasta_marker)
            .expect("Unable to read SFASTA Marker");
        assert!(
            sfasta_marker == "sfasta".as_bytes(),
            "File is missing sfasta magic bytes"
        );

        let mut sfasta = Sfasta {
            version: match bincode::decode_from_std_read(&mut in_buf, bincode_config) {
                Ok(x) => x,
                Err(y) => panic!("Error reading SFASTA directory: {}", y),
            },
            ..Default::default()
        };

        assert!(sfasta.version <= 1); // 1 is the maximum version supported at this stage...

        // TODO: In the future, with different versions, we will need to do different things
        // when we inevitabily introduce incompatabilities...

        let dir: DirectoryOnDisk = match bincode::decode_from_std_read(&mut in_buf, bincode_config)
        {
            Ok(x) => x,
            Err(y) => panic!("Error reading SFASTA directory: {}", y),
        };

        sfasta.directory = dir.into();

        sfasta.parameters = match bincode::decode_from_std_read(&mut in_buf, bincode_config) {
            Ok(x) => x,
            Err(y) => panic!("Error reading SFASTA parameters: {}", y),
        };

        sfasta.metadata = match bincode::decode_from_std_read(&mut in_buf, bincode_config) {
            Ok(x) => x,
            Err(y) => panic!("Error reading SFASTA metadata: {}", y),
        };

        // Next are the sequence blocks, which aren't important right now...
        // The index is much more important to us...

        // TODO: Handle no index
        sfasta.index = Some(DualIndex::new(
            &mut in_buf,
            sfasta.directory.index_loc.unwrap().get(),
        ));

        if sfasta.directory.seqlocs_loc.is_some() {
            let seqlocs_loc = sfasta.directory.seqlocs_loc.unwrap().get();
            let mut seqlocs = SeqLocs::from_buffer(&mut in_buf, seqlocs_loc);
            if prefetch {
                seqlocs.prefetch(&mut in_buf);
            }
            sfasta.seqlocs = Some(seqlocs);
        }

        in_buf
            .seek(SeekFrom::Start(
                sfasta.directory.block_index_loc.unwrap().get(),
            ))
            .expect("Unable to work with seek API");

        //let block_locs_compressed: Vec<u8> =
        //bincode::decode_from_std_read(&mut in_buf, bincode_config)
        //.expect("Unable to parse block locs index");
        let num_bits: u8 = bincode::decode_from_std_read(&mut in_buf, bincode_config)
            .expect("Unable to parse block locs index");

        let bitpacked_u32: Vec<Packed> = bincode::decode_from_std_read(&mut in_buf, bincode_config)
            .expect("Unable to parse block locs index");

        let block_locs_staggered = bitpacked_u32.into_iter().map(|x| x.unpack(num_bits));
        let block_locs_u32: Vec<u32> = block_locs_staggered.into_iter().flatten().collect();
        let block_locs: Vec<u64> = unsafe {
            std::slice::from_raw_parts(block_locs_u32.as_ptr() as *const u64, block_locs_u32.len())
                .to_vec()
        };

        std::mem::forget(block_locs_u32);
        sfasta.sequenceblocks = Some(SequenceBlocks::new(
            block_locs,
            sfasta.parameters.compression_type,
        ));

        if sfasta.directory.headers_loc.is_some() {
            let mut headers = Headers::from_buffer(
                &mut in_buf,
                sfasta.directory.headers_loc.unwrap().get() as u64,
            );
            if prefetch {
                headers.prefetch(&mut in_buf);
            }

            sfasta.headers = Some(headers);
        }

        if sfasta.directory.ids_loc.is_some() {
            let mut ids =
                Ids::from_buffer(&mut in_buf, sfasta.directory.ids_loc.unwrap().get() as u64);
            if prefetch {
                ids.prefetch(&mut in_buf);
            }
            sfasta.ids = Some(ids);
        }

        if sfasta.directory.masking_loc.is_some() {
            sfasta.masking = Some(Masking::from_buffer(
                &mut in_buf,
                sfasta.directory.masking_loc.unwrap().get() as u64,
            ));
        }

        sfasta.buf = Some(RwLock::new(Box::new(in_buf)));

        sfasta
    }
}

pub struct Sequences {
    sfasta: Sfasta,
    i: usize,
}

#[allow(dead_code)]
impl Sequences {
    pub fn new(sfasta: Sfasta) -> Sequences {
        Sequences { sfasta, i: 0 }
    }

    /// Convenience function. Likely to be less performant. Prefetch is off by default.
    pub fn from_file(path: &str) -> Sequences {
        Sequences::new(SfastaParser::open(path).expect("Unable to open file"))
    }
}

impl Iterator for Sequences {
    type Item = Sequence;

    fn next(&mut self) -> Option<Self::Item> {
        if self.sfasta.index.is_none() || self.i >= self.sfasta.len() {
            return None;
        }

        let seqloc = self.sfasta.seqlocs.as_ref().unwrap().data.as_ref().unwrap()[self.i].clone();

        let id = self.sfasta.get_id(seqloc.ids.as_ref().unwrap()).unwrap();
        let mut header = None;
        if seqloc.headers.is_some() {
            header = Some(
                self.sfasta
                    .get_header(seqloc.headers.as_ref().unwrap())
                    .expect("Unable to fetch header"),
            );
        }

        let sequence = self
            .sfasta
            .get_sequence(&seqloc)
            .expect("Unable to fetch sequence");

        // TODO: Scores

        self.i += 1;

        Some(Sequence {
            id,
            sequence,
            header,
            scores: None,
        })
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
        let mut out_buf = Box::new(Cursor::new(Vec::new()));

        let mut in_buf = BufReader::new(
            File::open("test_data/test_convert.fasta").expect("Unable to open testing file"),
        );

        let converter = Converter::default()
            .with_threads(6)
            .with_block_size(8 * 1024)
            .with_index();

        converter.convert_fasta(&mut in_buf, &mut out_buf);

        if let Err(x) = out_buf.seek(SeekFrom::Start(0)) {
            panic!("Unable to seek to start of file, {:#?}", x)
        };

        let mut sfasta = SfastaParser::open_from_buffer(out_buf, false);
        sfasta.index_len();

        assert_eq!(sfasta.index_len(), 3001);

        let output = sfasta.find("does-not-exist");
        assert!(output == Ok(None));

        let _output = &sfasta
            .find("needle")
            .expect("Unable to find-0")
            .expect("Unable to find-1");

        let output = &sfasta.find("needle_last").unwrap().unwrap();

        let sequence = sfasta.get_sequence(output).unwrap();
        let sequence = std::str::from_utf8(&sequence).unwrap();
        assert!("ACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATA" == sequence);
    }

    #[test]
    pub fn test_parse_multiple_blocks() {
        let mut out_buf = Box::new(Cursor::new(Vec::new()));

        let mut in_buf = BufReader::new(
            File::open("test_data/test_sequence_conversion.fasta")
                .expect("Unable to open testing file"),
        );

        let converter = Converter::default()
            .with_threads(6)
            .with_block_size(512)
            .with_index();

        converter.convert_fasta(&mut in_buf, &mut out_buf);

        if let Err(x) = out_buf.seek(SeekFrom::Start(0)) {
            panic!("Unable to seek to start of file, {:#?}", x)
        };

        let mut sfasta = SfastaParser::open_from_buffer(out_buf, false);
        assert!(sfasta.index_len() == 10);

        let output = &sfasta.find("test3").unwrap().unwrap();

        let sequence = sfasta.get_sequence(output).unwrap();
        let sequence = std::str::from_utf8(&sequence).unwrap();

        let sequence = sequence.trim();

        println!("{:#?}", &sequence[48590..]);
        println!("{:#?}", &sequence[48590..].as_bytes());

        assert!(&sequence[0..100] == "ATGCGATCCGCCCTTTCATGACTCGGGTCATCCAGCTCAATAACACAGACTATTTTATTGTTCTTCTTTGAAACCAGAACATAATCCATTGCCATGCCAT");
        assert!(&sequence[48000..48100] == "AACCGGCAGGTTGAATACCAGTATGACTGTTGGTTATTACTGTTGAAATTCTCATGCTTACCACCGCGGAATAACACTGGCGGTATCATGACCTGCCGGT");
        // Last 10
        let last_ten = sequence.len() - 10;
        assert!(&sequence[last_ten..] == "ATGTACAGCG");
        assert_eq!(sequence.len(), 48598);
    }

    #[test]
    pub fn test_find_does_not_trigger_infinite_loops() {
        let mut out_buf = Box::new(Cursor::new(Vec::new()));

        let mut in_buf = BufReader::new(
            File::open("test_data/test_sequence_conversion.fasta")
                .expect("Unable to open testing file"),
        );

        let converter = Converter::default()
            .with_threads(6)
            .with_block_size(512)
            .with_threads(8);

        converter.convert_fasta(&mut in_buf, &mut out_buf);

        if let Err(x) = out_buf.seek(SeekFrom::Start(0)) {
            panic!("Unable to seek to start of file, {:#?}", x)
        };

        let mut sfasta = SfastaParser::open_from_buffer(out_buf, false);
        assert!(sfasta.index_len() == 10);

        let _output = &sfasta.find("test3").unwrap().unwrap();
        sfasta.find("test").unwrap().unwrap();
        sfasta.find("test2").unwrap().unwrap();
        sfasta.find("test3").unwrap().unwrap();
        sfasta.find("test4").unwrap().unwrap();
        sfasta.find("test5").unwrap().unwrap();
    }
}
