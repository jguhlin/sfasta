use std::io::{Read, Seek, SeekFrom};
use std::sync::RwLock;

use rand::prelude::*;
use rand_chacha::ChaCha20Rng;

use crate::datatypes::*;
use crate::dual_level_index::*;
use crate::*;

/// Main Sfasta crate
pub struct Sfasta {
    pub version: u64, // I'm going to regret this, but 18,446,744,073,709,551,615 versions should be enough for anybody.
    pub directory: Directory,
    pub parameters: Parameters,
    pub metadata: Metadata,
    pub index_directory: IndexDirectory,
    pub index: Option<DualIndex>,
    buf: Option<RwLock<Box<dyn ReadAndSeek + Send>>>, // TODO: Needs to be behind RwLock to support better multi-threading...
    pub sequenceblocks: Option<SequenceBlocks<'static>>,
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

    pub fn get_block_size(&self) -> u32 {
        self.parameters.block_size
    }

    // TODO: Does nothing right now...
    pub fn compression_type(mut self, compression: CompressionType) -> Self {
        self.parameters.compression_type = compression;
        self
    }

    /// Get a Sequence object by ID.
    /// Convenience function. Not optimized for speed. If you don't need the header, scores, or masking,
    /// it's better to call more performant functions.
    pub fn get_sequence_by_id(&mut self, id: &str) -> Result<Option<Sequence>, &str> {
        let matches = self.find(id).expect("Unable to find entry");
        if matches == None {
            return Ok(None);
        }

        let matches = matches.unwrap();

        let id = if matches.ids.is_some() {
            Some(self.get_id(matches.ids.as_ref().unwrap()).unwrap())
        } else {
            None
        };

        let header = if matches.headers.is_some() {
            Some(self.get_header(matches.headers.as_ref().unwrap()).unwrap())
        } else {
            None
        };

        let sequence = if matches.sequence.is_some() {
            Some(self.get_sequence(&matches).unwrap())
        } else {
            None
        };

        /*
        // TODO
        todo!();
        let scores = if matches.scores.is_some() {
            Some(self.get_scores(&matches))
        } else {
            None
        }; */

        Ok(Some(Sequence {
            sequence,
            id,
            header,
            scores: None,
            offset: 0,
        }))
    }

    pub fn get_sequence_by_index(&mut self, idx: usize) -> Result<Option<Sequence>, &'static str> {
        let seqloc = match self.get_seqloc(idx) {
            Ok(Some(s)) => s,
            Ok(None) => return Ok(None),
            Err(e) => return Err(e),
        };

        seqloc
            .sequence
            .as_ref()
            .expect("No locations found, Vec<Loc> is empty");

        self.get_sequence_by_seqloc(&seqloc)
    }

    pub fn get_sequence_by_seqloc(
        &mut self,
        seqloc: &SeqLoc,
    ) -> Result<Option<Sequence>, &'static str> {
        let id = if seqloc.ids.is_some() {
            Some(self.get_id(seqloc.ids.as_ref().unwrap()).unwrap())
        } else {
            None
        };

        let header = if seqloc.headers.is_some() {
            Some(self.get_header(seqloc.headers.as_ref().unwrap()).unwrap())
        } else {
            None
        };

        let sequence = if seqloc.sequence.is_some() {
            Some(self.get_sequence(seqloc).unwrap())
        } else {
            None
        };

        /*
        // TODO
        todo!();
        let scores = if matches.scores.is_some() {
            Some(self.get_scores(&matches))
        } else {
            None
        }; */

        Ok(Some(Sequence {
            sequence,
            id,
            header,
            scores: None,
            offset: 0,
        }))
    }

    pub fn get_sequence_only_by_seqloc(
        &mut self,
        seqloc: &SeqLoc,
    ) -> Result<Option<Sequence>, &'static str> {
        let sequence = if seqloc.sequence.is_some() {
            Some(self.get_sequence(seqloc).unwrap())
        } else {
            None
        };

        Ok(Some(Sequence {
            sequence,
            id: None,
            header: None,
            scores: None,
            offset: 0,
        }))
    }

    // TODO: Should return Result<Option<Sequence>, &str>
    // TODO: Should actually be what get_sequence_by_seqloc is!
    pub fn get_sequence(&mut self, seqloc: &SeqLoc) -> Result<Vec<u8>, &'static str> {
        let mut seq: Vec<u8> = Vec::with_capacity(seqloc.len(self.parameters.block_size));

        assert!(seqloc.sequence.is_some());

        let locs = seqloc.sequence.as_ref().unwrap();

        let mut buf = self.buf.as_ref().unwrap().write().unwrap();

        println!("{:#?}", seqloc);

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
        seqlocs.get_seqloc(&mut buf, found.unwrap())
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

    pub fn len(&self) -> usize {
        self.seqlocs.as_ref().unwrap().len()
    }

    pub fn get_seqloc(&mut self, i: usize) -> Result<Option<SeqLoc>, &'static str> {
        assert!(i < self.len(), "Index out of bounds");
        assert!(i < std::u32::MAX as usize, "Index out of bounds");

        let mut buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        self.seqlocs
            .as_mut()
            .unwrap()
            .get_seqloc(&mut buf, i as u32)
    }

    /// Get all seqlocs
    pub fn get_seqlocs(&mut self) -> Result<Option<Vec<SeqLoc>>, &'static str> {
        let mut buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        self.seqlocs
            .as_mut()
            .unwrap()
            .prefetch(&mut buf);

        Ok(self.seqlocs.as_ref().unwrap().data.clone())
    }

    pub fn index_len(&mut self) -> usize {
        if self.index.is_none() {
            return 0;
        }

        self.index.as_mut().unwrap().len()
    }
}

pub struct SfastaParser {
    pub sfasta: Sfasta
}

impl SfastaParser {
    /// Convenience function to open a file and parse it.
    /// Prefetch defaults to false
    pub fn open(path: &str) -> Result<Sfasta, &'static str> {
        let in_buf = std::fs::File::open(path).expect("Unable to open file");
        let sfasta = SfastaParser::open_from_buffer(
            std::io::BufReader::new(in_buf),
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
            sfasta.parameters.block_size as usize,
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

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum SeqMode {
    Linear,
    Random,
}

impl Default for SeqMode {
    fn default() -> Self {
        SeqMode::Linear
    }
}

pub struct Sequences {
    sfasta: Sfasta,
    cur_idx: usize,
    mode: SeqMode,
    remaining_index: Option<Vec<usize>>,
    with_header: bool,
    with_scores: bool,
    with_ids: bool,
    with_sequences: bool,
    seed: Option<u64>,
}

#[allow(dead_code)]
impl Sequences {
    pub fn new(sfasta: Sfasta) -> Sequences {
        Sequences {
            sfasta,
            cur_idx: 0,
            mode: SeqMode::default(),
            remaining_index: None,
            with_header: false,
            with_scores: false,
            with_ids: true,
            with_sequences: true,
            seed: None,
        }
    }

    // Resets the iterator to the beginning of the sequences
    pub fn set_mode(&mut self, mode: SeqMode) {
        self.mode = mode;
        self.remaining_index = None;
        self.cur_idx = 0;
    }

    /// Convenience function. Likely to be less performant. Prefetch is off by default.
    pub fn from_file(path: &str) -> Sequences {
        Sequences::new(SfastaParser::open(path).expect("Unable to open file"))
    }

    pub fn with_header(mut self) -> Sequences {
        self.with_header = true;
        self
    }

    pub fn without_header(mut self) -> Sequences {
        self.with_header = false;
        self
    }

    pub fn with_scores(mut self) -> Sequences {
        self.with_scores = true;
        self
    }

    pub fn without_scores(mut self) -> Sequences {
        self.with_scores = false;
        self
    }

    pub fn with_seed(mut self, seed: u64) -> Sequences {
        self.seed = Some(seed);
        self
    }

    pub fn with_ids(mut self) -> Sequences {
        self.with_ids = true;
        self
    }

    pub fn without_ids(mut self) -> Sequences {
        self.with_ids = false;
        self
    }

    pub fn with_sequences(mut self) -> Sequences {
        self.with_sequences = true;
        self
    }

    pub fn without_sequences(mut self) -> Sequences {
        self.with_sequences = false;
        self
    }
}

impl Iterator for Sequences {
    type Item = Sequence;

    fn next(&mut self) -> Option<Self::Item> {
        if self.sfasta.index.is_none() || self.cur_idx >= self.sfasta.len() {
            return None;
        }

        if self.mode == SeqMode::Random {
            if self.remaining_index.is_none() {
                let mut rng = if let Some(seed) = self.seed {
                    ChaCha20Rng::seed_from_u64(seed)
                } else {
                    ChaCha20Rng::from_entropy()
                };

                let mut remaining_index: Vec<usize> = (0..self.sfasta.len()).collect();
                remaining_index.shuffle(&mut rng);
                self.remaining_index = Some(remaining_index);
            }

            let idx = match self.remaining_index.as_mut().unwrap().pop() {
                Some(idx) => idx,
                None => return None,
            };

            self.cur_idx = idx;
        }

        let seqloc =
            self.sfasta.get_seqloc(self.cur_idx).expect("Unable to get sequence location").expect(".");

        let id = if self.with_ids {
            Some(self.sfasta.get_id(seqloc.ids.as_ref().unwrap()).unwrap())
        } else {
            None
        };

        let header = if self.with_header && seqloc.headers.is_some() {
            Some(
                self.sfasta
                    .get_header(seqloc.headers.as_ref().unwrap())
                    .expect("Unable to fetch header"),
            )
        } else {
            None
        };

        /*
        let scores = if self.with_scores && seqloc.scores.is_some() {
            todo!();
        } else {
            None
        }; */

        let sequence = if self.with_sequences && seqloc.sequence.is_some() {
            Some(
                self.sfasta
                    .get_sequence(&seqloc)
                    .expect("Unable to fetch sequence"),
            )
        } else {
            None
        };

        // TODO: Scores

        if self.mode == SeqMode::Linear {
            self.cur_idx += 1;
        }

        Some(Sequence {
            id,
            sequence,
            header,
            scores: None,
            offset: 0,
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
