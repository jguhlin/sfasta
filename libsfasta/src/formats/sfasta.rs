use std::io::{Read, Seek, SeekFrom};
use std::sync::RwLock;

use rand::prelude::*;
use rand_chacha::ChaCha20Rng;

use crate::datatypes::*;
use crate::dual_level_index::*;
use crate::*;

/// Main Sfasta crate
pub struct Sfasta<'sfa> {
    pub version: u64, // I'm going to regret this, but 18,446,744,073,709,551,615 versions should be enough for anybody.
    pub directory: Directory,
    pub parameters: Parameters,
    pub metadata: Metadata,
    pub index_directory: IndexDirectory,
    pub index: Option<DualIndex>,
    buf: Option<RwLock<Box<dyn ReadAndSeek + Send + 'sfa>>>,
    pub sequenceblocks: Option<SequenceBlocks<'sfa>>,
    pub seqlocs: Option<SeqLocs<'sfa>>,
    pub headers: Option<StringBlockStore>,
    pub ids: Option<StringBlockStore>,
    pub masking: Option<Masking>,
}

impl<'sfa> Default for Sfasta<'sfa> {
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

impl<'sfa> Sfasta<'sfa> {
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
    // TODO: Support multiple matches
    pub fn get_sequence_by_id(&mut self, id: &str) -> Result<Option<Sequence>, &str> {
        let matches = self.find(id).expect("Unable to find entry");
        if matches.is_none() {
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
        cache: bool,
    ) -> Result<Option<Sequence>, &'static str> {
        let sequence = if seqloc.sequence.is_some() {
            if cache {
                Some(self.get_sequence(seqloc).unwrap())
            } else {
                Some(self.get_sequence_nocache(seqloc).unwrap())
            }
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

        let mut buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        let locs = seqloc.sequence.as_ref().unwrap();

        let seqlocs = self.seqlocs.as_mut().unwrap();
        let locs = seqlocs.get_locs(&mut buf, locs.0 as usize, locs.1 as usize);

        // Once stabilized, use write_all_vectored
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
            let masking = self.masking.as_mut().unwrap();
            let seqlocs = self.seqlocs.as_mut().unwrap();
            let locs = seqloc.masking.as_ref().unwrap();
            let locs = seqlocs.get_locs(&mut buf, locs.0 as usize, locs.1 as usize);
            masking.mask_sequence(&mut *buf, &locs, &mut seq);
        }

        Ok(seq)
    }

    pub fn get_sequence_nocache(&mut self, seqloc: &SeqLoc) -> Result<Vec<u8>, &'static str> {
        let mut seq: Vec<u8> = Vec::with_capacity(seqloc.len(self.parameters.block_size));

        assert!(seqloc.sequence.is_some());

        let mut buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        let locs = seqloc.sequence.as_ref().unwrap();

        let seqlocs = self.seqlocs.as_mut().unwrap();
        let locs = seqlocs.get_locs(&mut buf, locs.0 as usize, locs.1 as usize);

        for (block, (start, end)) in locs
            .iter()
            .map(|x| x.original_format(self.parameters.block_size))
        {
            let seqblock = self
                .sequenceblocks
                .as_mut()
                .unwrap()
                .get_block_uncached(&mut *buf, block);
            seq.extend_from_slice(&seqblock[start as usize..end as usize]);
        }

        if seqloc.masking.is_some() && self.masking.is_some() {
            let masking = self.masking.as_mut().unwrap();
            let seqlocs = self.seqlocs.as_mut().unwrap();
            let locs = seqloc.masking.as_ref().unwrap();
            let locs = seqlocs.get_locs(&mut buf, locs.0 as usize, locs.1 as usize);
            masking.mask_sequence(&mut *buf, &locs, &mut seq);
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

    pub fn get_header(&mut self, locs: &(u64, u8)) -> Result<String, &'static str> {
        let headers = self.headers.as_mut().unwrap();
        let mut buf = &mut *self.buf.as_ref().unwrap().write().unwrap();

        let seqlocs = self.seqlocs.as_mut().unwrap();
        let locs = seqlocs.get_locs(&mut buf, locs.0 as usize, locs.1 as usize);

        Ok(headers.get(&mut buf, &locs))
    }

    pub fn get_id(&mut self, locs: &(u64, u8)) -> Result<String, &'static str> {
        let mut buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        let seqlocs = self.seqlocs.as_mut().unwrap();
        let locs = seqlocs.get_locs(&mut buf, locs.0 as usize, locs.1 as usize);

        let ids = self.ids.as_mut().unwrap();

        Ok(ids.get(&mut buf, &locs))
    }

    pub fn len(&self) -> usize {
        self.seqlocs.as_ref().unwrap().total_seqlocs
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
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
    pub fn get_seqlocs(&mut self) -> Result<Option<&Vec<SeqLoc>>, &'static str> {
        let mut buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        self.seqlocs.as_mut().unwrap().prefetch(&mut buf);

        // TODO: Fail is index is not initialized yet (prefetch does it here, but still)
        Ok(self
            .seqlocs
            .as_mut()
            .unwrap()
            .get_all_seqlocs(&mut buf)
            .unwrap())
    }

    pub fn index_len(&mut self) -> usize {
        if self.index.is_none() {
            return 0;
        }

        self.index.as_mut().unwrap().len()
    }
}

pub struct SfastaParser<'sfa> {
    pub sfasta: Sfasta<'sfa>,
}

impl<'sfa> SfastaParser<'sfa> {
    /// Convenience function to open a file and parse it.
    /// Prefetch defaults to false
    pub fn open(path: String) -> Result<Sfasta<'sfa>, String> {
        let in_buf = std::fs::File::open(path).expect("Unable to open file");
        SfastaParser::open_from_buffer(in_buf, false)
    }

    // TODO: Can probably multithread parts of this...
    // Prefetch should probably be another name...
    pub fn open_from_buffer<R>(mut in_buf: R, prefetch: bool) -> Result<Sfasta<'sfa>, String>
    where
        R: 'sfa + Read + Seek + Send,
    {
        let bincode_config = bincode::config::standard()
            .with_fixed_int_encoding()
            .with_limit::<{ 2 * 1024 * 1024 }>();

        let mut sfasta_marker: [u8; 6] = [0; 6];
        match in_buf.read_exact(&mut sfasta_marker) {
            Ok(_) => (),
            Err(x) => return Result::Err(format!("Invalid file. {}", x)),
        };

        log::info!("Sfasta marker: {:?}", sfasta_marker);

        #[cfg(not(fuzzing))]
        assert!(
            sfasta_marker == "sfasta".as_bytes(),
            "File is missing sfasta magic bytes"
        );

        let mut sfasta = Sfasta {
            version: match bincode::decode_from_std_read(&mut in_buf, bincode_config) {
                Ok(x) => x,
                Err(y) => return Result::Err(format!("Error reading SFASTA directory: {}", y)),
            },
            ..Default::default()
        };

        #[cfg(not(fuzzing))]
        assert!(sfasta.version <= 1); // 1 is the maximum version supported at this stage...

        // TODO: In the future, with different versions, we will need to do different things
        // when we inevitabily introduce incompatabilities...

        log::info!("Parsing DirectoryOnDisk");

        let dir: DirectoryOnDisk = match bincode::decode_from_std_read(&mut in_buf, bincode_config)
        {
            Ok(x) => x,
            Err(y) => return Result::Err(format!("Error reading SFASTA directory: {}", y)),
        };

        sfasta.directory = dir.into();

        log::info!("Parsing Parameters");

        sfasta.parameters = match bincode::decode_from_std_read(&mut in_buf, bincode_config) {
            Ok(x) => x,
            Err(y) => return Result::Err(format!("Error reading SFASTA parameters: {}", y)),
        };

        log::info!("Parsing Metadata");
        sfasta.metadata = match bincode::decode_from_std_read(&mut in_buf, bincode_config) {
            Ok(x) => x,
            Err(y) => return Result::Err(format!("Error reading SFASTA metadata: {}", y)),
        };

        // Next are the sequence blocks, which aren't important right now...
        // The index is much more important to us...

        let bincode_config = bincode::config::standard()
            .with_fixed_int_encoding()
            .with_limit::<{ 128 * 1024 * 1024 }>();

        log::info!("Index");
        // TODO: Handle no index
        if sfasta.directory.index_loc.is_some() {
            sfasta.index =
                match DualIndex::new(&mut in_buf, sfasta.directory.index_loc.unwrap().get()) {
                    Ok(x) => Some(x),
                    Err(y) => return Result::Err(format!("Error reading SFASTA index: {}", y)),
                };
        } else {
            return Result::Err(
                "No index found in SFASTA file - Support for no index is not yet implemented."
                    .to_string(),
            );
        }

        log::info!("SeqLocs");

        if sfasta.directory.seqlocs_loc.is_some() {
            let seqlocs_loc = sfasta.directory.seqlocs_loc.unwrap().get();
            let mut seqlocs = match SeqLocs::from_buffer(&mut in_buf, seqlocs_loc) {
                Ok(x) => x,
                Err(y) => return Result::Err(format!("Error reading SFASTA seqlocs: {}", y)),
            };

            if prefetch {
                seqlocs.prefetch(&mut in_buf);
            }
            sfasta.seqlocs = Some(seqlocs);
        }

        log::info!("Parsing Blocks");

        if sfasta.directory.block_index_loc.is_none() {
            return Result::Err("No block index found in SFASTA file - Support for no block index is not yet implemented.".to_string());
        }

        in_buf
            .seek(SeekFrom::Start(
                sfasta.directory.block_index_loc.unwrap().get(),
            ))
            .expect("Unable to work with seek API");

        let num_bits: u8 = match bincode::decode_from_std_read(&mut in_buf, bincode_config) {
            Ok(x) => x,
            Err(y) => return Result::Err(format!("Error reading SFASTA block index: {}", y)),
        };

        let x: (u64, u64) = match bincode::decode_from_std_read(&mut in_buf, bincode_config) {
            Ok(x) => x,
            Err(y) => return Result::Err(format!("Error reading SFASTA block index: {}", y)),
        };

        // TODO: Not yet used, but eliminates the need to read all the block locs into memory (thus speeding up large files such as nt)
        let compressed_size = x.0;
        let blocks_count = x.1;

        log::info!("Num Bits: {}", num_bits);
        log::info!("Compressed Size: {}", compressed_size);
        log::info!("Blocks Counts: {}", blocks_count);

        if num_bits > 32 {
            return Result::Err(format!("Invalid num bits: {}", num_bits));
        }

        let block_index_loc = in_buf.seek(SeekFrom::Current(0)).unwrap();
        println!("Block index loc: {}", block_index_loc);

        log::info!("Creating Sequence Blocks");

        sfasta.sequenceblocks = Some(SequenceBlocks::new(
            sfasta.parameters.compression_type,
            sfasta.parameters.compression_dict.clone(),
            sfasta.parameters.block_size as usize,
            compressed_size,
            blocks_count,
            block_index_loc,
            num_bits,
        ));

        println!("Prefetch: {}", prefetch);

        if prefetch {
            println!("Prefetching block locs");
            sfasta
                .sequenceblocks
                .as_mut()
                .unwrap()
                .prefetch_block_locs(&mut in_buf)
                .expect("Unable to prefetch block locs");
        }

        log::info!("Opening Headers");
        if sfasta.directory.headers_loc.is_some() {
            let mut headers = match StringBlockStore::from_buffer(
                &mut in_buf,
                sfasta.directory.headers_loc.unwrap().get(),
            ) {
                Ok(x) => x,
                Err(y) => return Result::Err(format!("Error reading SFASTA headers: {}", y)),
            };

            if prefetch {
                headers.prefetch(&mut in_buf);
            }

            sfasta.headers = Some(headers);
        }

        log::info!("Opening IDs");
        if sfasta.directory.ids_loc.is_some() {
            let mut ids = match StringBlockStore::from_buffer(
                &mut in_buf,
                sfasta.directory.ids_loc.unwrap().get(),
            ) {
                Ok(x) => x,
                Err(y) => return Result::Err(format!("Error reading SFASTA ids: {}", y)),
            };
            if prefetch {
                ids.prefetch(&mut in_buf);
            }
            sfasta.ids = Some(ids);
        }

        log::info!("Opening Masking");
        if sfasta.directory.masking_loc.is_some() {
            sfasta.masking = match Masking::from_buffer(
                &mut in_buf,
                sfasta.directory.masking_loc.unwrap().get(),
            ) {
                Ok(x) => Some(x),
                Err(y) => return Result::Err(format!("Error reading SFASTA masking: {}", y)),
            };
        }

        log::info!("Storing buf");
        sfasta.buf = Some(RwLock::new(Box::new(in_buf)));

        log::info!("Done!");
        Ok(sfasta)
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

pub struct Sequences<'sfa> {
    sfasta: Sfasta<'sfa>,
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
impl<'sfa> Sequences<'sfa> {
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
    pub fn from_file(path: String) -> Sequences<'sfa> {
        Sequences::new(SfastaParser::open(path).expect("Unable to open file"))
    }

    pub fn with_header(mut self) -> Sequences<'sfa> {
        self.with_header = true;
        self
    }

    pub fn without_header(mut self) -> Sequences<'sfa> {
        self.with_header = false;
        self
    }

    pub fn with_scores(mut self) -> Sequences<'sfa> {
        self.with_scores = true;
        self
    }

    pub fn without_scores(mut self) -> Sequences<'sfa> {
        self.with_scores = false;
        self
    }

    pub fn with_seed(mut self, seed: u64) -> Sequences<'sfa> {
        self.seed = Some(seed);
        self
    }

    pub fn with_ids(mut self) -> Sequences<'sfa> {
        self.with_ids = true;
        self
    }

    pub fn without_ids(mut self) -> Sequences<'sfa> {
        self.with_ids = false;
        self
    }

    pub fn with_sequences(mut self) -> Sequences<'sfa> {
        self.with_sequences = true;
        self
    }

    pub fn without_sequences(mut self) -> Sequences<'sfa> {
        self.with_sequences = false;
        self
    }
}

impl<'sfa> Iterator for Sequences<'sfa> {
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

        let seqloc = self
            .sfasta
            .get_seqloc(self.cur_idx)
            .expect("Unable to get sequence location")
            .expect(".");

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

        #[cfg(miri)]
        let mut converter = converter.with_compression_type(CompressionType::NONE);

        println!("Converting file...");
        converter.convert_fasta(&mut in_buf, &mut out_buf);

        if let Err(x) = out_buf.seek(SeekFrom::Start(0)) {
            panic!("Unable to seek to start of file, {:#?}", x)
        };

        println!("Opening file...");
        let mut sfasta = SfastaParser::open_from_buffer(out_buf, false).unwrap();
        sfasta.index_len();

        println!("Checking index len...");
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

        // TODO: Test this with prefecth both true and false...
        let mut sfasta = SfastaParser::open_from_buffer(out_buf, false).unwrap();
        assert!(sfasta.index_len() == 10);

        let output = &sfasta.find("test").unwrap().unwrap();
        println!("'test' seqloc: {:#?}", output);
        let sequence = sfasta.get_sequence(output).unwrap();
        println!("'test' Sequence length: {}", sequence.len());

        let output = &sfasta.find("test3").unwrap().unwrap();
        println!("'test3' seqloc: {:#?}", output);

        let sequence = sfasta.get_sequence(output).unwrap();
        let sequence = std::str::from_utf8(&sequence).unwrap();

        let sequence = sequence.trim();

        // println!("{:#?}", sequence);

        println!("'test3' Sequence length: {}", sequence.len());
        let last_ten = sequence.len() - 10;
        // println!("{:#?}", &sequence[last_ten..].as_bytes());
        println!("{:#?}", &sequence[last_ten..]);

        println!("{:#?}", &sequence[0..100]);
        assert!(&sequence[0..100] == "ATGCGATCCGCCCTTTCATGACTCGGGTCATCCAGCTCAATAACACAGACTATTTTATTGTTCTTCTTTGAAACCAGAACATAATCCATTGCCATGCCAT");
        assert!(&sequence[48000..48100] == "AACCGGCAGGTTGAATACCAGTATGACTGTTGGTTATTACTGTTGAAATTCTCATGCTTACCACCGCGGAATAACACTGGCGGTATCATGACCTGCCGGT");
        // Last 10

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

        let mut sfasta = SfastaParser::open_from_buffer(out_buf, false).unwrap();
        assert!(sfasta.index_len() == 10);

        let _output = &sfasta.find("test3").unwrap().unwrap();
        sfasta.find("test").unwrap().unwrap();
        sfasta.find("test2").unwrap().unwrap();
        sfasta.find("test3").unwrap().unwrap();
        sfasta.find("test4").unwrap().unwrap();
        sfasta.find("test5").unwrap().unwrap();
    }
}
