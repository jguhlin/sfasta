use std::io::{Read, Seek, SeekFrom};
use std::sync::RwLock;

use rayon::prelude::*;

use crate::data_types::*;
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
    pub block_locs: Option<Vec<u64>>,
    pub seqlocs: Option<SeqLocs>,
    pub headers: Option<Headers>,
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
            seqlocs: None,
            headers: None,
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

    pub fn get_sequence(&self, locs: &Vec<Loc>) -> Result<Vec<u8>, &'static str> {
        let mut seq: Vec<u8> = Vec::with_capacity(2 * 1024 * 1024); // TODO: We can calculate this

        if locs.is_empty() {
            return Err("No locations passed, Vec<Loc> is empty");
        }

        // Basic sanity checks
        for (i, _) in locs.iter().map(|x| x.original_format()) {
            if i as usize >= self.block_locs.as_ref().unwrap().len() {
                return Err("Requested block number is larger than the total number of blocks");
            }
        }

        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        for loc in locs {
            let byte_loc = self.block_locs.as_ref().unwrap()[loc.block as usize];
            let mut buf = self.buf.as_ref().unwrap().write().unwrap();

            buf.seek(SeekFrom::Start(byte_loc))
                .expect("Unable to work with seek API");

            let sbc: SequenceBlockCompressed =
                bincode::decode_from_std_read(&mut *buf, bincode_config)
                    .expect("Unable to parse SequenceBlockCompressed");

            drop(buf); // Open it up for other threads...
                       // let sb = bump.alloc(sbc.decompress(self.parameters.compression_type));
            let sb = sbc.decompress(self.parameters.compression_type);

            seq.extend_from_slice(&sb.seq[loc.start as usize..loc.end as usize]);
            // bump.reset();
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
    // TODO: Can probably multithread parts of this...
    pub fn open_from_buffer<R>(mut in_buf: R) -> Sfasta
    where
        R: 'static + Read + Seek + Send,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        let mut sfasta_marker: [u8; 6] = [0; 6];
        in_buf
            .read_exact(&mut sfasta_marker)
            .expect("Unable to read SFASTA Marker");
        assert!(sfasta_marker == "sfasta".as_bytes());

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
            let seqlocs = SeqLocs::from_buffer(&mut in_buf, seqlocs_loc);
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
        sfasta.block_locs = Some(block_locs);

        if sfasta.directory.headers_loc.is_some() {
            sfasta.headers = Some(Headers::from_buffer(
                &mut in_buf,
                sfasta.directory.headers_loc.unwrap().get() as u64,
            ));
        }

        sfasta.buf = Some(RwLock::new(Box::new(in_buf)));

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
        let out_buf = Cursor::new(Vec::new());

        let in_buf = BufReader::new(
            File::open("test_data/test_convert.fasta").expect("Unable to open testing file"),
        );

        let converter = Converter::default()
            .with_threads(6)
            .with_block_size(8 * 1024)
            .with_index();

        let mut out_buf = converter.convert_fasta(in_buf, out_buf);

        if let Err(x) = out_buf.seek(SeekFrom::Start(0)) {
            panic!("Unable to seek to start of file, {:#?}", x)
        };

        let mut sfasta = SfastaParser::open_from_buffer(out_buf);
        sfasta.index_len();

        assert_eq!(sfasta.index_len(), 3001);

        let output = sfasta.find("does-not-exist");
        assert!(output == Ok(None));

        let output = &sfasta.find("needle").unwrap().unwrap();
        assert!(output.id == "needle");

        let output = &sfasta.find("needle_last").unwrap().unwrap();
        assert!(output.id == "needle_last");

        let sequence = sfasta
            .get_sequence(output.sequence.as_ref().unwrap())
            .unwrap();
        let sequence = std::str::from_utf8(&sequence).unwrap();
        assert!("ACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATA" == sequence);
    }

    #[test]
    pub fn test_parse_multiple_blocks() {
        let out_buf = Cursor::new(Vec::new());

        let in_buf = BufReader::new(
            File::open("test_data/test_sequence_conversion.fasta")
                .expect("Unable to open testing file"),
        );

        let converter = Converter::default()
            .with_threads(6)
            .with_block_size(512)
            .with_index();

        let mut out_buf = converter.convert_fasta(in_buf, out_buf);

        if let Err(x) = out_buf.seek(SeekFrom::Start(0)) {
            panic!("Unable to seek to start of file, {:#?}", x)
        };

        let mut sfasta = SfastaParser::open_from_buffer(out_buf);
        assert!(sfasta.index_len() == 10);

        let output = &sfasta.find("test3").unwrap().unwrap();
        assert!(output.id == "test3");

        let sequence = sfasta
            .get_sequence(output.sequence.as_ref().unwrap())
            .unwrap();
        let sequence = std::str::from_utf8(&sequence).unwrap();

        assert_eq!(sequence.len(), 48598);
        assert!(&sequence[0..100] == "ATGCGATCCGCCCTTTCATGACTCGGGTCATCCAGCTCAATAACACAGACTATTTTATTGTTCTTCTTTGAAACCAGAACATAATCCATTGCCATGCCAT");
        assert!(&sequence[48000..48100] == "AACCGGCAGGTTGAATACCAGTATGACTGTTGGTTATTACTGTTGAAATTCTCATGCTTACCACCGCGGAATAACACTGGCGGTATCATGACCTGCCGGT");
    }

    #[test]
    pub fn test_find_does_not_trigger_infinite_loops() {
        let out_buf = Cursor::new(Vec::new());

        let in_buf = BufReader::new(
            File::open("test_data/test_sequence_conversion.fasta")
                .expect("Unable to open testing file"),
        );

        let converter = Converter::default()
            .with_threads(6)
            .with_block_size(512)
            .with_threads(8);

        let mut out_buf = converter.convert_fasta(in_buf, out_buf);

        if let Err(x) = out_buf.seek(SeekFrom::Start(0)) {
            panic!("Unable to seek to start of file, {:#?}", x)
        };

        let mut sfasta = SfastaParser::open_from_buffer(out_buf);
        assert!(sfasta.index_len() == 10);

        let output = &sfasta.find("test3").unwrap().unwrap();
        assert!(output.id == "test3");
        sfasta.find("test").unwrap().unwrap();
        sfasta.find("test2").unwrap().unwrap();
        sfasta.find("test3").unwrap().unwrap();
        sfasta.find("test4").unwrap().unwrap();
        sfasta.find("test5").unwrap().unwrap();
    }
}
