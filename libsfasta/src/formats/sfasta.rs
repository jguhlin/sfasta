//! Structs to open and work with SFASTA file format
//!
//! This module contains the main methods of reading SFASTA files, including iterators
//! to iterate over contained sequences.

use std::{
    io::{Read, Seek, SeekFrom},
    sync::RwLock,
};

use libcompression::*;
use libfractaltree::FractalTreeRead;

use rand::prelude::*;
use rand_chacha::ChaCha20Rng;
use xxhash_rust::xxh3::xxh3_64;

use crate::{datatypes::*, *};

/// Main Sfasta struct
pub struct Sfasta<'sfa>
{
    pub version: u64, /* I'm going to regret this, but 18,446,744,073,709,551,615 versions should be enough for
                       * anybody. */
    pub directory: Directory,
    pub parameters: Parameters,
    pub metadata: Metadata,
    pub index_directory: IndexDirectory,
    pub index: Option<FractalTreeRead>,
    buf: Option<RwLock<Box<dyn ReadAndSeek + Send + Sync + 'sfa>>>,
    pub sequenceblocks: Option<SequenceBlockStore>,
    pub seqlocs: Option<SeqLocsStore>,
    pub headers: Option<StringBlockStore>,
    pub ids: Option<StringBlockStore>,
    pub masking: Option<Masking>,
}

impl<'sfa> Default for Sfasta<'sfa>
{
    fn default() -> Self
    {
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

impl<'sfa> Sfasta<'sfa>
{
    /// Use for after cloning(primarily for multiple threads), give the object a new read buffer
    pub fn with_buffer<R>(mut self, buf: R) -> Self
    where
        R: 'sfa + Read + Seek + Send + Sync,
    {
        self.buf = Some(RwLock::new(Box::new(buf)));
        self
    }

    pub fn with_sequences(self) -> Self
    {
        // TODO: Should we really have SFASTA without sequences?!
        // self.directory = self.directory.with_sequences();
        self
    }

    pub fn with_scores(mut self) -> Self
    {
        self.directory = self.directory.with_scores();
        self
    }

    pub fn with_masking(mut self) -> Self
    {
        self.directory = self.directory.with_masking();
        self
    }

    pub fn block_size(mut self, block_size: u32) -> Self
    {
        self.parameters.block_size = block_size;
        self
    }

    pub const fn get_block_size(&self) -> u32
    {
        self.parameters.block_size
    }

    // TODO: Does nothing right now...
    pub fn compression_type(mut self, compression: CompressionType) -> Self
    {
        self.parameters.compression_type = compression;
        self
    }

    pub fn seq_slice(&mut self, seqloc: &SeqLoc, range: std::ops::Range<u32>) -> Vec<Loc>
    {
        let block_size = self.get_block_size();

        seqloc.seq_slice(block_size, range)
    }

    /// Get a Sequence object by ID.
    /// Convenience function. Not optimized for speed. If you don't need the header, scores, or masking,
    /// it's better to call more performant functions.
    // TODO: Support multiple matches
    pub fn get_sequence_by_id(&mut self, id: &str) -> Result<Option<Sequence>, &str>
    {
        let matches = self.find(id).expect("Unable to find entry");
        if matches.is_none() {
            return Ok(None);
        }

        let matches = matches.unwrap();

        let id = if matches.ids > 0 {
            Some(self.get_id(matches.get_ids()).unwrap())
        } else {
            None
        };

        let header = if matches.headers > 0 {
            Some(self.get_header(matches.get_headers()).unwrap())
        } else {
            None
        };

        let sequence = if matches.sequence > 0 {
            Some(self.get_sequence(&matches).unwrap())
        } else {
            None
        };

        // TODO
        // todo!();
        // let scores = if matches.scores.is_some() {
        // Some(self.get_scores(&matches))
        // } else {
        // None
        // };

        Ok(Some(Sequence {
            sequence,
            id,
            header,
            scores: None,
            offset: 0,
        }))
    }

    pub fn get_sequence_by_index(&mut self, idx: usize) -> Result<Option<Sequence>, &'static str>
    {
        let seqloc = match self.get_seqloc(idx) {
            Ok(Some(s)) => s,
            Ok(None) => return Ok(None),
            Err(e) => return Err(e),
        };

        assert!(seqloc.sequence > 0);

        self.get_sequence_by_seqloc(&seqloc)
    }

    pub fn get_sequence_by_seqloc(&mut self, seqloc: &SeqLoc) -> Result<Option<Sequence>, &'static str>
    {
        let id = if seqloc.ids > 0 {
            Some(self.get_id(seqloc.get_ids()).unwrap())
        } else {
            None
        };

        let header = if seqloc.headers > 0 {
            Some(self.get_header(seqloc.get_headers()).unwrap())
        } else {
            None
        };

        let sequence = if seqloc.sequence > 0 {
            Some(self.get_sequence(seqloc).unwrap())
        } else {
            None
        };

        // TODO
        // todo!();
        // let scores = if matches.scores.is_some() {
        // Some(self.get_scores(&matches))
        // } else {
        // None
        // };

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
    ) -> Result<Option<Sequence>, &'static str>
    {
        let sequence = if seqloc.sequence > 0 {
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

    pub fn get_sequence_only_by_locs(&mut self, locs: &[Loc], cache: bool) -> Result<Option<Sequence>, &'static str>
    {
        let sequence = if cache {
            Some(self.get_sequence_by_locs(locs).unwrap())
        } else {
            Some(self.get_sequence_by_locs_nocache(locs).unwrap())
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
    pub fn get_sequence(&mut self, seqloc: &SeqLoc) -> Result<Vec<u8>, &'static str>
    {
        assert!(seqloc.sequence > 0);

        let buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        let locs = seqloc.get_sequence();

        let mut seq: Vec<u8> = Vec::new();

        // Once stabilized, use write_all_vectored
        for l in locs.iter().map(|x| x) {
            let seqblock = self.sequenceblocks.as_mut().unwrap().get_block(&mut *buf, l.block);
            seq.extend_from_slice(&seqblock[l.start as usize..(l.start + l.len) as usize]);
        }

        if seqloc.masking > 0 && self.masking.is_some() {
            let masking = self.masking.as_mut().unwrap();
            let locs = seqloc.get_masking();
            masking.mask_sequence(&mut *buf, &locs, &mut seq);
        }

        Ok(seq)
    }

    pub fn get_sequence_nocache(&mut self, seqloc: &SeqLoc) -> Result<Vec<u8>, &'static str>
    {
        let mut seq: Vec<u8> = Vec::with_capacity(1024);

        assert!(seqloc.sequence > 0);

        let buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        let locs = seqloc.get_sequence();

        for l in locs.iter().map(|x| x) {
            let seqblock = self
                .sequenceblocks
                .as_mut()
                .unwrap()
                .get_block_uncached(&mut *buf, l.block);
            seq.extend_from_slice(&seqblock[l.start as usize..(l.start + l.len) as usize]);
        }

        if seqloc.masking > 0 && self.masking.is_some() {
            let masking = self.masking.as_mut().unwrap();
            let locs = seqloc.get_masking();
            masking.mask_sequence(&mut *buf, &locs, &mut seq);
        }

        Ok(seq)
    }

    // No masking is possible here...
    pub fn get_sequence_by_locs(&mut self, locs: &[Loc]) -> Result<Vec<u8>, &'static str>
    {
        let mut seq: Vec<u8> = Vec::with_capacity(1024);

        let buf = &mut *self.buf.as_ref().unwrap().write().unwrap();

        // Once stabilized, use write_all_vectored
        for l in locs.iter().map(|x| x) {
            let seqblock = self.sequenceblocks.as_mut().unwrap().get_block(&mut *buf, l.block);
            seq.extend_from_slice(&seqblock[l.start as usize..(l.start + l.len) as usize]);
        }

        Ok(seq)
    }

    // No masking is possible here...
    pub fn get_sequence_by_locs_nocache(&mut self, locs: &[Loc]) -> Result<Vec<u8>, &'static str>
    {
        let mut seq: Vec<u8> = Vec::with_capacity(256);

        let buf = &mut *self.buf.as_ref().unwrap().write().unwrap();

        for l in locs.iter().map(|x| x) {
            let seqblock = self
                .sequenceblocks
                .as_mut()
                .unwrap()
                .get_block_uncached(&mut *buf, l.block);
            seq.extend_from_slice(&seqblock[l.start as usize..(l.start + l.len) as usize]);
        }

        Ok(seq)
    }

    pub fn find(&mut self, x: &str) -> Result<Option<SeqLoc>, &str>
    {
        assert!(self.index.is_some(), "Sfasta index not present");

        let idx = self.index.as_mut().unwrap();
        let mut buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        let key = xxh3_64(x.as_bytes());
        let found = idx.search(key);
        let seqlocs = self.seqlocs.as_mut().unwrap();

        if found.is_none() {
            return Ok(None);
        }

        // TODO: Allow returning multiple if there are multiple matches...
        log::debug!("Getting seqloc");
        seqlocs.get_seqloc(&mut buf, found.unwrap())
    }

    pub fn get_header(&mut self, locs: &[Loc]) -> Result<String, &'static str>
    {
        let headers = self.headers.as_mut().unwrap();
        let mut buf = &mut *self.buf.as_ref().unwrap().write().unwrap();

        Ok(headers.get(&mut buf, &locs))
    }

    pub fn get_id(&mut self, locs: &[Loc]) -> Result<String, &'static str>
    {
        let mut buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        let ids = self.ids.as_mut().unwrap();
        Ok(ids.get(&mut buf, &locs))
    }

    pub fn get_id_loaded(&self, locs: &[Loc]) -> Result<String, &'static str>
    {
        let ids = self.ids.as_ref().unwrap();

        Ok(ids.get_loaded(&locs))
    }

    // TODO
    // pub fn len(&self) -> usize {
    // self.seqlocs.as_ref().unwrap().data.len()
    // }

    // Get length from SeqLoc (only for sequence)
    pub fn seqloc_len(&mut self, seqloc: &SeqLoc) -> usize
    {
        seqloc.len()
    }

    // Get length from SeqLoc (only for sequence)
    // Immutable for preloaded datasets
    // TODO
    // pub fn seqloc_len_loaded(&self, seqloc: &SeqLoc) -> usize {
    // let seqlocs = self.seqlocs.as_ref().unwrap();
    // seqloc.len_loaded(seqlocs, self.parameters.block_size)
    // }
    //
    // pub fn is_empty(&self) -> bool {
    // self.len() == 0
    // }

    /// Get the ith seqloc in the file
    pub fn get_seqloc(&mut self, i: usize) -> Result<Option<SeqLoc>, &'static str>
    {
        // assert!(i < self.len(), "Index out of bounds");
        assert!(i < std::u32::MAX as usize, "Index out of bounds");

        let mut buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        self.seqlocs.as_mut().unwrap().get_seqloc(&mut buf, i as u32)
    }

    /// Get all seqlocs
    pub fn get_seqlocs(&mut self) -> Result<Option<&Vec<SeqLoc>>, &'static str>
    {
        let mut buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        self.seqlocs.as_mut().unwrap().prefetch(&mut buf);

        // TODO: Fail if index is not initialized yet (prefetch does it here, but still)
        Ok(self.seqlocs.as_mut().unwrap().get_all_seqlocs(&mut buf).unwrap())
    }

    /// This is more expensive than getting it from the seqlocs
    pub fn index_len(&self) -> usize
    {
        if self.index.is_none() {
            #[cfg(test)]
            println!("Index is none");

            return 0;
        }

        self.index.as_ref().unwrap().len()
    }
}

pub struct SfastaParser<'sfa>
{
    pub sfasta: Sfasta<'sfa>,
}

impl<'sfa> SfastaParser<'sfa>
{
    /// Convenience function to open a file and parse it.
    /// Does not prefetch indices automatically
    ///
    /// ```no_run
    /// # use libsfasta::prelude::*;
    /// let sfasta = SfastaParser::open("myfile.sfasta").unwrap();
    /// ```
    pub fn open<P: AsRef<std::path::Path>>(path: P) -> Result<Sfasta<'sfa>, String>
    {
        // Check file size is reasonable:
        let metadata = std::fs::metadata(&path).unwrap();
        if metadata.len() < 128 {
            // 128 is a guess, but should figure out the header size....
            return Err("File size is too small to be a valid SFASTA".to_string());
        }

        let in_buf =
            std::fs::File::open(&path).unwrap_or_else(|_| panic!("Unable to open file: {}", path.as_ref().display()));
        SfastaParser::open_from_buffer(in_buf, false)
    }

    // TODO: Can probably multithread parts of this...
    // Prefetch should probably be another name...
    pub fn open_from_buffer<R>(mut in_buf: R, prefetch: bool) -> Result<Sfasta<'sfa>, String>
    where
        R: 'sfa + Read + Seek + Send + Sync,
    {
        let bincode_config = bincode::config::standard()
            .with_fixed_int_encoding()
            .with_limit::<{ 2 * 1024 * 1024 }>();

        // Confirm buffer is a reasonable size. 64 is random but maybe acceptable size...
        let buffer_length = in_buf.seek(SeekFrom::End(0)).unwrap();
        if buffer_length < 128 {
            return Result::Err(format!("File is too small to be a valid SFASTA"));
        }

        in_buf.seek(SeekFrom::Start(0)).unwrap();

        let mut sfasta_marker: [u8; 6] = [0; 6];
        match in_buf.read_exact(&mut sfasta_marker) {
            Ok(_) => (),
            Err(x) => return Result::Err(format!("Invalid file. {x}")),
        };

        log::info!("Sfasta marker: {:?}", sfasta_marker);

        if sfasta_marker != "sfasta".as_bytes() {
            return Result::Err(format!("Invalid SFASTA Format. File is missing sfasta magic bytes."));
        }

        let mut sfasta = Sfasta {
            version: match bincode::decode_from_std_read(&mut in_buf, bincode_config) {
                Ok(x) => x,
                Err(y) => return Result::Err(format!("Error reading SFASTA directory: {y}")),
            },
            ..Default::default()
        };

        if sfasta.version != 1 {
            return Result::Err(format!(
                "Invalid SFASTA Format. File is version {} but this library only supports version 1",
                sfasta.version
            ));
        }

        // TODO: In the future, with different versions, we will need to do different things
        // when we inevitabily introduce incompatabilities...

        log::info!("Parsing DirectoryOnDisk");

        let dir: DirectoryOnDisk = match bincode::decode_from_std_read(&mut in_buf, bincode_config) {
            Ok(x) => x,
            Err(y) => return Result::Err(format!("Error reading SFASTA directory: {y}")),
        };

        match dir.sanity_check(buffer_length) {
            Ok(_) => (),
            Err(x) => return Result::Err(format!("Invalid SFASTA directory: {x}")),
        }

        sfasta.directory = dir.into();

        log::info!("Parsing Parameters");

        sfasta.parameters = match bincode::decode_from_std_read(&mut in_buf, bincode_config) {
            Ok(x) => x,
            Err(y) => return Result::Err(format!("Error reading SFASTA parameters: {y}")),
        };

        log::info!("Parsing Metadata");
        sfasta.metadata = match bincode::decode_from_std_read(&mut in_buf, bincode_config) {
            Ok(x) => x,
            Err(y) => return Result::Err(format!("Error reading SFASTA metadata: {y}")),
        };

        // Next are the sequence blocks, which aren't important right now...
        // The index is much more important to us...

        log::info!("Loading Index");
        // TODO: Handle no index
        // TODO: Update
        if sfasta.directory.index_loc.is_some() {
            sfasta.index = match DualIndex::new(&mut in_buf, sfasta.directory.index_loc.unwrap().get()) {
                Ok(x) => Some(x),
                Err(y) => return Result::Err(format!("Error reading SFASTA index: {y}")),
            };
        } else {
            return Result::Err(
                "No index found in SFASTA file - Support for no index is not yet implemented.".to_string(),
            );
        }

        log::info!("SeqLocs");

        if sfasta.directory.seqlocs_loc.is_some() {
            let seqlocs_loc = sfasta.directory.seqlocs_loc.unwrap().get();
            let mut seqlocs = match SeqLocsStore::from_buffer(&mut in_buf, seqlocs_loc) {
                Ok(x) => x,
                Err(y) => return Result::Err(format!("Error reading SFASTA seqlocs: {y}")),
            };

            if prefetch {
                seqlocs.prefetch(&mut in_buf);
            }
            sfasta.seqlocs = Some(seqlocs);
        }

        log::info!("Parsing Blocks");

        // if sfasta.directory.block_index_loc.is_none() {
        // return Result::Err("No block index found in SFASTA file - Support for no block index is not yet
        // implemented.".to_string()); }

        // in_buf
        //.seek(SeekFrom::Start(
        // sfasta.directory.block_index_loc.unwrap().get(),
        //))
        //    .expect("Unable to work with seek API");

        // let block_index_loc = in_buf.stream_position().unwrap();

        log::info!("Creating Sequence Blocks");

        let sequenceblocks = SequenceBlockStore::from_buffer(&mut in_buf, sfasta.directory.masking_loc.unwrap().get());

        if sequenceblocks.is_err() {
            return Result::Err(format!(
                "Error reading SFASTA sequence blocks: {:?}",
                sequenceblocks.err()
            ));
        }

        sfasta.sequenceblocks = Some(sequenceblocks.unwrap());

        if prefetch {
            todo!("Prefetching block locs is not yet implemented");
            println!("Prefetching block locs");
            //            sfasta
            //              .sequenceblocks
            //            .as_mut()
            //          .unwrap()
            // .prefetch_block_locs(&mut in_buf) // TODO: Important to speed things back up
            // .expect("Unable to prefetch block locs");
        }

        log::info!("Opening Headers");
        if sfasta.directory.headers_loc.is_some() {
            let mut headers =
                match StringBlockStore::from_buffer(&mut in_buf, sfasta.directory.headers_loc.unwrap().get()) {
                    Ok(x) => x,
                    Err(y) => return Result::Err(format!("Error reading SFASTA headers - StringBlockStore: {y}")),
                };

            if prefetch {
                headers.prefetch(&mut in_buf);
            }

            sfasta.headers = Some(headers);
        }

        log::info!("Opening IDs");
        if sfasta.directory.ids_loc.is_some() {
            let mut ids = match StringBlockStore::from_buffer(&mut in_buf, sfasta.directory.ids_loc.unwrap().get()) {
                Ok(x) => x,
                Err(y) => return Result::Err(format!("Error reading SFASTA ids: {y}")),
            };
            if prefetch {
                ids.prefetch(&mut in_buf);
            }
            sfasta.ids = Some(ids);
        }

        log::info!("Opening Masking");
        if sfasta.directory.masking_loc.is_some() {
            sfasta.masking = match Masking::from_buffer(&mut in_buf, sfasta.directory.masking_loc.unwrap().get()) {
                Ok(x) => Some(x),
                Err(y) => return Result::Err(format!("Error reading SFASTA masking: {y}")),
            };
        }

        log::info!("Storing buf");
        sfasta.buf = Some(RwLock::new(Box::new(in_buf)));

        log::info!("Done!");
        Ok(sfasta)
    }
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum SeqMode
{
    Linear,
    Random,
}

impl Default for SeqMode
{
    fn default() -> Self
    {
        SeqMode::Linear
    }
}

pub struct Sequences<'sfa>
{
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
impl<'sfa> Sequences<'sfa>
{
    pub fn new(sfasta: Sfasta) -> Sequences
    {
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
    pub fn set_mode(&mut self, mode: SeqMode)
    {
        self.mode = mode;
        self.remaining_index = None;
        self.cur_idx = 0;
    }

    /// Convenience function. Likely to be less performant. Prefetch is off by default.
    pub fn from_file(path: String) -> Sequences<'sfa>
    {
        Sequences::new(SfastaParser::open(path).expect("Unable to open file"))
    }

    pub fn with_header(mut self) -> Sequences<'sfa>
    {
        self.with_header = true;
        self
    }

    pub fn without_header(mut self) -> Sequences<'sfa>
    {
        self.with_header = false;
        self
    }

    pub fn with_scores(mut self) -> Sequences<'sfa>
    {
        self.with_scores = true;
        self
    }

    pub fn without_scores(mut self) -> Sequences<'sfa>
    {
        self.with_scores = false;
        self
    }

    pub fn with_seed(mut self, seed: u64) -> Sequences<'sfa>
    {
        self.seed = Some(seed);
        self
    }

    pub fn with_ids(mut self) -> Sequences<'sfa>
    {
        self.with_ids = true;
        self
    }

    pub fn without_ids(mut self) -> Sequences<'sfa>
    {
        self.with_ids = false;
        self
    }

    pub fn with_sequences(mut self) -> Sequences<'sfa>
    {
        self.with_sequences = true;
        self
    }

    pub fn without_sequences(mut self) -> Sequences<'sfa>
    {
        self.with_sequences = false;
        self
    }
}

impl<'sfa> Iterator for Sequences<'sfa>
{
    type Item = Sequence;

    fn next(&mut self) -> Option<Self::Item>
    {
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

        let sequence = if self.with_sequences && seqloc.sequence.is_some() {
            Some(self.sfasta.get_sequence(&seqloc).expect("Unable to fetch sequence"))
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
mod tests
{
    use super::*;
    use crate::conversion::Converter;
    use std::{
        fs::File,
        io::{BufReader, Cursor},
    };

    #[test]
    pub fn test_sfasta_find_and_retrieve_sequence()
    {
        let mut out_buf = Box::new(Cursor::new(Vec::new()));

        let mut in_buf =
            BufReader::new(File::open("test_data/test_convert.fasta").expect("Unable to open testing file"));

        let converter = Converter::default()
            .with_threads(6)
            .with_block_size(8 * 1024)
            .with_index();

        #[cfg(miri)]
        let mut converter = converter.with_compression_type(CompressionType::NONE);

        let mut out_buf = converter.convert(&mut in_buf, out_buf);

        if let Err(x) = out_buf.seek(SeekFrom::Start(0)) {
            panic!("Unable to seek to start of file, {x:#?}")
        };

        let mut sfasta = SfastaParser::open_from_buffer(out_buf, false).unwrap();
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
    pub fn test_parse_multiple_blocks()
    {
        let out_buf = Box::new(Cursor::new(Vec::new()));

        let mut in_buf = BufReader::new(
            File::open("test_data/test_sequence_conversion.fasta").expect("Unable to open testing file"),
        );

        let converter = Converter::default().with_threads(6).with_block_size(512).with_index();

        let mut out_buf = converter.convert(&mut in_buf, out_buf);

        if let Err(x) = out_buf.seek(SeekFrom::Start(0)) {
            panic!("Unable to seek to start of file, {x:#?}")
        };

        // TODO: Test this with prefecth both true and false...
        let mut sfasta = SfastaParser::open_from_buffer(out_buf, false).unwrap();
        assert!(sfasta.index_len() == 10);

        let output = &sfasta.find("test").unwrap().unwrap();
        println!("'test' seqloc: {output:#?}");
        let sequence = sfasta.get_sequence(output).unwrap();
        println!("'test' Sequence length: {}", sequence.len());

        let output = &sfasta.find("test3").unwrap().unwrap();
        println!("'test3' seqloc: {output:#?}");

        let sequence = sfasta.get_sequence(output).unwrap();
        let sequence = std::str::from_utf8(&sequence).unwrap();

        let sequence = sequence.trim();

        // println!("{:#?}", sequence);

        println!("'test3' Sequence length: {}", sequence.len());
        let last_ten = sequence.len() - 10;
        // println!("{:#?}", &sequence[last_ten..].as_bytes());
        println!("{:#?}", &sequence[last_ten..]);

        println!("{:#?}", &sequence[0..100]);
        assert!(
      &sequence[0..100]
        == "ATGCGATCCGCCCTTTCATGACTCGGGTCATCCAGCTCAATAACACAGACTATTTTATTGTTCTTCTTTGAAACCAGAACATAATCCATTGCCATGCCAT"
    );
        assert!(
      &sequence[48000..48100]
        == "AACCGGCAGGTTGAATACCAGTATGACTGTTGGTTATTACTGTTGAAATTCTCATGCTTACCACCGCGGAATAACACTGGCGGTATCATGACCTGCCGGT"
    );
        // Last 10

        assert!(&sequence[last_ten..] == "ATGTACAGCG");
        assert_eq!(sequence.len(), 48598);
    }

    #[test]
    pub fn test_find_does_not_trigger_infinite_loops()
    {
        env_logger::init();
        let mut out_buf = Box::new(Cursor::new(Vec::new()));

        let mut in_buf = BufReader::new(
            File::open("test_data/test_sequence_conversion.fasta").expect("Unable to open testing file"),
        );

        let converter = Converter::default().with_block_size(512).with_threads(1);

        let mut out_buf = converter.convert(&mut in_buf, out_buf);

        if let Err(x) = out_buf.seek(SeekFrom::Start(0)) {
            panic!("Unable to seek to start of file, {x:#?}")
        };

        let mut sfasta = SfastaParser::open_from_buffer(out_buf, false).unwrap();
        println!("Index len: {:#?}", sfasta.index_len());
        assert!(sfasta.index_len() == 10);

        let _output = &sfasta.find("test3").unwrap().unwrap();

        sfasta.find("test").unwrap().unwrap();
        sfasta.find("test2").unwrap().unwrap();
        sfasta.find("test3").unwrap().unwrap();
        sfasta.find("test4").unwrap().unwrap();
        sfasta.find("test5").unwrap().unwrap();
    }
}
