//! Structs to open and work with SFASTA file format
//!
//! This module contains the main methods of reading SFASTA files,
//! including iterators to iterate over contained sequences.

use std::{
    io::{BufRead, Read, Seek, SeekFrom},
    sync::RwLock,
    sync::Arc,
};

use libfractaltree::FractalTreeDisk;

use rand::prelude::*;
use rand_chacha::ChaCha20Rng;
use xxhash_rust::xxh3::xxh3_64;

use crate::datatypes::*;

/// Main Sfasta struct
pub struct Sfasta<'sfa>
{
    pub version: u64, /* I'm going to regret this, but
                       * 18,446,744,073,709,551,615 versions should be
                       * enough for anybody.
                       *
                       * todo u128 is stable now
                       */
    pub directory: Directory,
    pub metadata: Option<Metadata>,
    pub index: Option<FractalTreeDisk<u32, u32>>,
    pub buf: Option<RwLock<Box<dyn ReadAndSeek + Send + Sync + 'sfa>>>,
    pub sequences: Option<BytesBlockStore>,
    pub seqlocs: Option<SeqLocsStore>,
    pub headers: Option<StringBlockStore>,
    pub ids: Option<StringBlockStore>,
    pub masking: Option<Masking>,
    pub scores: Option<BytesBlockStore>,
    pub file: Option<String>,

    // For async mode we have shared file handles
    #[cfg(feature = "async")]
    pub file_handles: Option<
        Arc<tokio::sync::RwLock<Vec<Arc<tokio::sync::RwLock<tokio::io::BufReader<tokio::fs::File>>>>>>,
    >,
}

impl<'sfa> Default for Sfasta<'sfa>
{
    fn default() -> Self
    {
        Sfasta {
            version: 1,
            directory: Directory::default(),
            metadata: None,
            index: None,
            buf: None,
            sequences: None,
            seqlocs: None,
            headers: None,
            ids: None,
            masking: None,
            file: None,
            scores: None,

            #[cfg(feature = "async")]
            file_handles: None,
        }
    }
}

impl<'sfa> Sfasta<'sfa>
{
    pub fn conversion(mut self) -> Self
    {
        self.metadata = Some(Metadata::default());
        self
    }

    /// Use for after cloning(primarily for multiple threads), give
    /// the object a new read buffer
    pub fn with_buffer<R>(mut self, buf: R) -> Self
    where
        R: 'sfa + Read + Seek + Send + Sync + BufRead,
    {
        self.buf = Some(RwLock::new(Box::new(buf)));
        self
    }

    pub fn seq_slice(
        &mut self,
        seqloc: &SeqLoc,
        range: std::ops::Range<u32>,
    ) -> Vec<Loc>
    {
        let block_size = self.sequences.as_ref().unwrap().block_size;

        seqloc.seq_slice(block_size, range)
    }

    /// Get a Sequence object by ID.
    /// Convenience function. Not optimized for speed. If you don't
    /// need the header, scores, or masking, it's better to call
    /// more performant functions.
    // TODO: Support multiple matches
    pub fn get_sequence_by_id(
        &mut self,
        id: &str,
    ) -> Result<Option<Sequence>, &str>
    {
        let matches = self.find(id).expect("Unable to find entry");
        if matches.is_none() {
            return Ok(None);
        }

        let matches = matches.unwrap().clone();

        let id = if matches.ids > 0 {
            Some(self.get_id(matches.get_ids()).unwrap().into_bytes())
        // todo
        } else {
            None
        };

        let header = if matches.headers > 0 {
            Some(self.get_header(matches.get_headers()).unwrap().into())
        // todo
        } else {
            None
        };

        let sequence = if matches.sequence > 0 {
            Some(
                self.get_sequence(
                    &matches.get_sequence(),
                    &matches.get_masking(),
                )
                .unwrap(),
            )
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

    pub fn get_sequence_by_index(
        &mut self,
        idx: usize,
    ) -> Result<Option<Sequence>, &'static str>
    {
        let seqloc = match self.get_seqloc(idx) {
            Ok(Some(s)) => s,
            Ok(None) => return Ok(None),
            Err(e) => return Err(e),
        }
        .clone();

        assert!(seqloc.sequence > 0);

        self.get_sequence_by_seqloc(&seqloc)
    }

    pub fn get_sequence_by_seqloc(
        &mut self,
        seqloc: &SeqLoc,
    ) -> Result<Option<Sequence>, &'static str>
    {
        let id = if seqloc.ids > 0 {
            Some(self.get_id(seqloc.get_ids()).unwrap().into_bytes()) // todo
        } else {
            None
        };

        let header = if seqloc.headers > 0 {
            Some(self.get_header(seqloc.get_headers()).unwrap().into())
        // todo
        } else {
            None
        };

        let sequence = if seqloc.sequence > 0 {
            Some(
                self.get_sequence(seqloc.get_sequence(), seqloc.get_masking())
                    .unwrap(),
            )
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
                Some(
                    self.get_sequence(
                        seqloc.get_sequence(),
                        seqloc.get_masking(),
                    )
                    .unwrap(),
                )
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

    pub fn get_sequence_only_by_locs(
        &mut self,
        locs: &[Loc],
        cache: bool,
    ) -> Result<Option<Sequence>, &'static str>
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
    /// Gets the sequence specified with seqloc, and applies masking
    /// specified with maskingloc.. To have no masking just pass a
    /// blank slice.
    pub fn get_sequence(
        &mut self,
        seqloc: &[Loc],
        maskingloc: &[Loc],
    ) -> Result<Vec<u8>, &'static str>
    {
        let buf = &mut *self.buf.as_ref().unwrap().write().unwrap();

        let mut seq: Vec<u8> = Vec::new();

        seqloc.iter().for_each(|l| {
            let seqblock = self
                .sequences
                .as_mut()
                .unwrap()
                .get_block(&mut *buf, l.block);
            seq.extend_from_slice(
                &seqblock[l.start as usize..(l.start + l.len) as usize],
            );
        });

        if !maskingloc.is_empty() && self.masking.is_some() {
            let masking = self.masking.as_mut().unwrap();
            masking.mask_sequence(&mut *buf, maskingloc, &mut seq);
        }

        Ok(seq)
    }

    pub fn get_sequence_nocache(
        &mut self,
        seqloc: &SeqLoc,
    ) -> Result<Vec<u8>, &'static str>
    {
        let mut seq: Vec<u8> = Vec::new();

        assert!(seqloc.sequence > 0);

        let buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        let locs = seqloc.get_sequence();

        let mut buffer =
            vec![0u8; self.sequences.as_ref().unwrap().block_size as usize];

        locs.iter().for_each(|l| {
            self.sequences.as_mut().unwrap().get_block_uncached(
                &mut *buf,
                l.block,
                &mut buffer,
            );
            seq.extend_from_slice(
                &buffer[l.start as usize..(l.start + l.len) as usize],
            );
        });

        if seqloc.masking > 0 && self.masking.is_some() {
            let masking = self.masking.as_mut().unwrap();
            let locs = seqloc.get_masking();
            masking.mask_sequence(&mut *buf, &locs, &mut seq);
        }

        Ok(seq)
    }

    // No masking is possible here...
    pub fn get_sequence_by_locs(
        &mut self,
        locs: &[Loc],
    ) -> Result<Vec<u8>, &'static str>
    {
        let mut seq: Vec<u8> = Vec::with_capacity(1024);

        let buf = &mut *self.buf.as_ref().unwrap().write().unwrap();

        // Once stabilized, use write_all_vectored
        for l in locs.iter().map(|x| x) {
            let seqblock = self
                .sequences
                .as_mut()
                .unwrap()
                .get_block(&mut *buf, l.block);
            seq.extend_from_slice(
                &seqblock[l.start as usize..(l.start + l.len) as usize],
            );
        }

        Ok(seq)
    }

    // No masking is possible here...
    pub fn get_sequence_by_locs_nocache(
        &mut self,
        locs: &[Loc],
    ) -> Result<Vec<u8>, &'static str>
    {
        let mut seq: Vec<u8> = Vec::with_capacity(256);

        let buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        let mut buffer =
            vec![0u8; self.sequences.as_ref().unwrap().block_size()];

        locs.iter().for_each(|l| {
            self.sequences.as_mut().unwrap().get_block_uncached(
                &mut *buf,
                l.block,
                &mut buffer,
            );
            seq.extend_from_slice(
                &buffer[l.start as usize..(l.start + l.len) as usize],
            );
        });

        Ok(seq)
    }

    pub fn find(&mut self, x: &str) -> Result<Option<SeqLoc>, &str>
    {
        assert!(self.index.is_some(), "Sfasta index not present");

        let idx = self.index.as_mut().unwrap();
        let mut buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        let key = xxh3_64(x.as_bytes());
        let found = idx.search(&mut buf, &(key as u32));
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

    pub fn len(&mut self) -> usize
    {
        let mut buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        self.seqlocs.as_mut().unwrap().len(&mut buf)
    }

    // Get length from SeqLoc (only for sequence)
    pub fn seqloc_len(&mut self, seqloc: &SeqLoc) -> usize
    {
        seqloc.len()
    }

    /// Get the ith seqloc in the file
    pub fn get_seqloc(
        &mut self,
        i: usize,
    ) -> Result<Option<SeqLoc>, &'static str>
    {
        // assert!(i < self.len(), "Index out of bounds");
        assert!(i < std::u32::MAX as usize, "Index out of bounds");

        let mut buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        self.seqlocs
            .as_mut()
            .unwrap()
            .get_seqloc(&mut buf, i as u32)
    }

    /// Get all seqlocs
    pub fn get_seqlocs(&mut self) -> Result<(), &'static str>
    {
        let mut buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        self.seqlocs.as_mut().unwrap().prefetch(&mut buf);

        // TODO: Fail if index is not initialized yet (prefetch does it here,
        // but still)
        self.seqlocs.as_mut().unwrap().get_all_seqlocs(&mut buf)
    }

    /// This is more expensive than getting it from the seqlocs
    pub fn index_len(&self) -> Result<usize, &'static str>
    {
        if self.index.is_none() {
            return Err("Index does not exist");
        }

        self.index.as_ref().unwrap().len()
    }

    pub fn index_load(&mut self) -> Result<(), &'static str>
    {
        let buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        self.index.as_mut().unwrap().load_tree(buf)
    }
}

#[cfg(feature = "async")]
impl<'sfa> Sfasta<'sfa> {
    pub async fn get_filehandle(&self) -> tokio::sync::OwnedRwLockWriteGuard<tokio::io::BufReader<tokio::fs::File>> {
        let file_handles = Arc::clone(self.file_handles.as_ref().unwrap());

        loop {
            let file_handles_read = file_handles.read().await;

            // Loop through each until we find one that is not locked
            for file_handle in file_handles_read.iter() {
                let file_handle = Arc::clone(file_handle);
                let file_handle = file_handle.try_write_owned();
                if let Ok(file_handle) = file_handle {
                    return file_handle;
                }
            }

            if file_handles_read.len() < 256 {
                // There are no available file handles, so we need to create a new one
                // and the number isn't crazy (yet)
                break;
            }

            drop(file_handles_read);

            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }

        // Otherwise, create one and add it to the list
        let file = tokio::fs::File::open(self.file.as_ref().unwrap()).await.unwrap();
        let file_handle = std::sync::Arc::new(tokio::sync::RwLock::new(tokio::io::BufReader::new(file)));

        let mut write_lock = file_handles.write().await;

        write_lock.push(Arc::clone(&file_handle));

        file_handle.try_write_owned().unwrap()       
    }

    pub async fn find_asnyc(&mut self, x: &str) -> Result<Option<SeqLoc>, &str> {
        assert!(self.index.is_some(), "Sfasta index not present");

        let mut buf = self.get_filehandle().await;
        let idx = self.index.as_mut().unwrap();
        let key = xxh3_64(x.as_bytes());
        let found = idx.search(&mut buf, &(key as u32)).await;
        let seqlocs = self.seqlocs.as_mut().unwrap();

        if found.is_none() {
            return Ok(None);
        }

        // todo return multiple matches
        log::debug!("Getting seqloc");
        log::debug!("Found seqloc: {:?}", found.unwrap());
        unimplemented!();
        // seqlocs.get_seqloc_async(&mut buf, found.unwrap())
    }
}

pub struct SfastaParser<'sfa>
{
    pub sfasta: Sfasta<'sfa>,
}

impl<'sfa> SfastaParser<'sfa>
{
    /// Convenience function to open a file and parse it.
    ///
    /// ```no_run
    /// # use libsfasta::prelude::*;
    /// let sfasta = SfastaParser::open("myfile.sfasta").unwrap();
    /// ```
    pub fn open<P: AsRef<std::path::Path>>(
        path: P,
    ) -> Result<Sfasta<'sfa>, String>
    {
        // Check file size is reasonable:
        let metadata = std::fs::metadata(&path).unwrap();
        if metadata.len() < 128 {
            // 128 is a guess, but should figure out the header size....
            return Err(
                "File size is too small to be a valid SFASTA".to_string()
            );
        }

        let in_buf = std::fs::File::open(&path).unwrap_or_else(|_| {
            panic!("Unable to open file: {}", path.as_ref().display())
        });
        let in_buf = std::io::BufReader::new(in_buf);
        SfastaParser::open_from_buffer(in_buf)
    }

    // TODO: Can probably multithread parts of this...
    // Prefetch should probably be another name...
    pub fn open_from_buffer<R>(mut in_buf: R) -> Result<Sfasta<'sfa>, String>
    where
        R: 'sfa + Read + Seek + Send + Sync + BufRead,
    {
        let bincode_config_fixed = crate::BINCODE_CONFIG
            .with_fixed_int_encoding()
            .with_limit::<{ 2 * 1024 * 1024 }>();

        // Confirm buffer is a reasonable size. 128 is random but maybe
        // acceptable size...
        let buffer_length = in_buf.seek(SeekFrom::End(0)).unwrap();
        if buffer_length < 128 {
            return Result::Err(format!(
                "File is too small to be a valid SFASTA"
            ));
        }

        in_buf.seek(SeekFrom::Start(0)).unwrap();

        let mut sfasta_marker: [u8; 6] = [0; 6];
        match in_buf.read_exact(&mut sfasta_marker) {
            Ok(_) => (),
            Err(x) => return Result::Err(format!("Invalid file. {x}")),
        };

        if sfasta_marker != "sfasta".as_bytes() {
            return Result::Err(format!(
                "Invalid SFASTA Format. File is missing sfasta magic bytes."
            ));
        }

        let mut sfasta = Sfasta {
            version: match bincode::decode_from_std_read(
                &mut in_buf,
                bincode_config_fixed,
            ) {
                Ok(x) => x,
                Err(y) => {
                    return Result::Err(format!(
                        "Error reading SFASTA directory: {y}"
                    ))
                }
            },
            ..Default::default()
        };

        if sfasta.version != 1 {
            return Result::Err(format!(
                "Invalid SFASTA Format. File is version {} but this library only supports version 1",
                sfasta.version
            ));
        }

        // TODO: In the future, with different versions, we will need to do
        // different things when we inevitabily introduce
        // incompatabilities...

        log::info!("Parsing DirectoryOnDisk");

        let dir: DirectoryOnDisk = match bincode::decode_from_std_read(
            &mut in_buf,
            bincode_config_fixed,
        ) {
            Ok(x) => x,
            Err(y) => {
                return Result::Err(format!(
                    "Error reading SFASTA directory: {y}"
                ))
            }
        };

        sfasta.directory = dir.into();

        log::info!("Loading Index");
        if sfasta.directory.index_loc.is_some() {
            let tree = FractalTreeDisk::from_buffer(
                &mut in_buf,
                sfasta.directory.index_loc.unwrap().get(),
            )
            .unwrap();
            sfasta.index = Some(tree);
        } else {
            sfasta.index = None;
        }

        log::info!("SeqLocs");
        if sfasta.directory.seqlocs_loc.is_some() {
            let seqlocs_loc = sfasta.directory.seqlocs_loc.unwrap().get();
            let seqlocs =
                match SeqLocsStore::from_existing(seqlocs_loc, &mut in_buf) {
                    Ok(x) => x,
                    Err(y) => {
                        return Result::Err(format!(
                            "Error reading SFASTA seqlocs: {y}"
                        ))
                    }
                };
            sfasta.seqlocs = Some(seqlocs);
        }

        let sequenceblocks = BytesBlockStore::from_buffer(
            &mut in_buf,
            sfasta.directory.sequences_loc.unwrap().get(),
        );

        if sequenceblocks.is_err() {
            return Result::Err(format!(
                "Error reading SFASTA sequence blocks: {:?}",
                sequenceblocks.err()
            ));
        }

        sfasta.sequences = Some(sequenceblocks.unwrap());

        log::info!("Opening Headers");
        if sfasta.directory.headers_loc.is_some() {
            let headers = match StringBlockStore::from_buffer(
                &mut in_buf,
                sfasta.directory.headers_loc.unwrap().get(),
            ) {
                Ok(x) => x,
                Err(y) => {
                    return Result::Err(format!(
                        "Error reading SFASTA headers - StringBlockStore: {y}"
                    ))
                }
            };

            sfasta.headers = Some(headers);
        }

        log::info!("Opening IDs");
        if sfasta.directory.ids_loc.is_some() {
            let ids = match StringBlockStore::from_buffer(
                &mut in_buf,
                sfasta.directory.ids_loc.unwrap().get(),
            ) {
                Ok(x) => x,
                Err(y) => {
                    return Result::Err(format!(
                        "Error reading SFASTA ids: {y}"
                    ))
                }
            };
            sfasta.ids = Some(ids);
        }

        log::info!("Opening Masking");
        if sfasta.directory.masking_loc.is_some() {
            sfasta.masking = match Masking::from_buffer(
                &mut in_buf,
                sfasta.directory.masking_loc.unwrap().get(),
            ) {
                Ok(x) => Some(x),
                Err(y) => {
                    return Result::Err(format!(
                        "Error reading SFASTA masking: {y}"
                    ))
                }
            };
        }

        log::info!("Storing buf");
        sfasta.buf = Some(RwLock::new(Box::new(in_buf)));

        log::info!("File open complete");
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

    /// Convenience function. Likely to be less performant. Prefetch
    /// is off by default.
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

                let mut remaining_index: Vec<usize> =
                    (0..self.sfasta.len()).collect();
                remaining_index.shuffle(&mut rng);
                self.remaining_index = Some(remaining_index);
            }

            let idx = match self.remaining_index.as_mut().unwrap().pop() {
                Some(idx) => idx,
                None => return None,
            };

            self.cur_idx = idx;
        }

        // TODO this is wrong (shouldn't be cur_idx)
        let seqloc = self
            .sfasta
            .get_seqloc(self.cur_idx)
            .expect("Unable to get sequence location")
            .expect(".")
            .clone();

        let id = if self.with_ids {
            Some(self.sfasta.get_id(seqloc.get_ids()).unwrap().into_bytes())
        // todo
        } else {
            None
        };

        let header = seqloc.get_headers();

        let header = if header.len() > 0 {
            Some(
                self.sfasta
                    .get_header(header)
                    .expect("Unable to fetch header")
                    .into_bytes(),
            ) // todo
        } else {
            None
        };

        let sequences = seqloc.get_sequence();
        let maskingloc = seqloc.get_masking();

        let sequence = if !sequences.is_empty() {
            Some(
                self.sfasta
                    .get_sequence(&sequences, &maskingloc)
                    .expect("Unable to fetch sequence"),
            )
        } else {
            None
        };

        // TODO: Scores

        // TODO: Signal

        if self.mode == SeqMode::Linear {
            self.cur_idx += 1;
        }

        // todo: shortcircuiting Sequence when reading files so leaving id and
        // header as bytes but here we want to return it as a str
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
        let _ = env_logger::builder().is_test(true).try_init();
        let out_buf = Box::new(Cursor::new(Vec::new()));

        println!("1...");

        let mut in_buf = BufReader::new(
            File::open("test_data/test_convert.fasta")
                .expect("Unable to open testing file"),
        );

        let mut converter = Converter::default();
        converter
            .with_threads(6)
            .with_block_size(8 * 1024)
            .with_index();

        let mut out_buf = converter.convert(&mut in_buf, out_buf);

        println!("2...");

        if let Err(x) = out_buf.seek(SeekFrom::Start(0)) {
            panic!("Unable to seek to start of file, {x:#?}")
        };

        println!("3...");

        let mut sfasta = SfastaParser::open_from_buffer(out_buf).unwrap();
        println!("4...");
        sfasta.index_load().expect("Unable to load index");
        println!("5...");

        assert_eq!(sfasta.index_len(), Ok(3001));

        println!("Got here");

        let output = sfasta.find("does-not-exist");
        assert!(output == Ok(None));

        let _output = &sfasta
            .find("needle")
            .expect("Unable to find-0")
            .expect("Unable to find-1");

        let output = &sfasta.find("needle_last").unwrap().unwrap().clone();

        let sequence = sfasta
            .get_sequence(output.get_sequence(), output.get_masking())
            .unwrap();
        let sequence = std::str::from_utf8(&sequence).unwrap();
        assert!("ACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATAACTGGGGGNAATTATATA" == sequence);
    }

    #[test]
    pub fn test_parse_multiple_blocks()
    {
        let _ = env_logger::builder().is_test(true).try_init();
        let out_buf = Box::new(Cursor::new(Vec::new()));

        let mut in_buf = BufReader::new(
            File::open("test_data/test_sequence_conversion.fasta")
                .expect("Unable to open testing file"),
        );

        let mut converter = Converter::default();
        converter.with_threads(6).with_block_size(512).with_index();

        let mut out_buf = converter.convert(&mut in_buf, out_buf);

        if let Err(x) = out_buf.seek(SeekFrom::Start(0)) {
            panic!("Unable to seek to start of file, {x:#?}")
        };

        // TODO: Test this with prefecth both true and false...
        let mut sfasta = SfastaParser::open_from_buffer(out_buf).unwrap();
        sfasta.index_load().expect("Unable to load index");
        assert!(sfasta.index_len() == Ok(10));

        let output = &sfasta.find("test").unwrap().unwrap().clone();
        println!("'test' seqloc: {output:#?}");
        let sequence = sfasta
            .get_sequence(output.get_sequence(), output.get_masking())
            .unwrap();
        println!("'test' Sequence length: {}", sequence.len());

        let output = &sfasta.find("test3").unwrap().unwrap().clone();
        println!("'test3' seqloc: {output:#?}");

        let sequence = sfasta
            .get_sequence(output.get_sequence(), output.get_masking())
            .unwrap();
        let sequence = std::str::from_utf8(&sequence).unwrap();

        let sequence = sequence.trim();

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
        let mut out_buf = Box::new(Cursor::new(Vec::new()));

        let mut in_buf = BufReader::new(
            File::open("test_data/test_sequence_conversion.fasta")
                .expect("Unable to open testing file"),
        );

        let mut converter = Converter::default();
        converter.with_block_size(512).with_threads(1);

        let mut out_buf = converter.convert(&mut in_buf, out_buf);

        if let Err(x) = out_buf.seek(SeekFrom::Start(0)) {
            panic!("Unable to seek to start of file, {x:#?}")
        };

        let mut sfasta = SfastaParser::open_from_buffer(out_buf).unwrap();
        sfasta.index_load().expect("Unable to load index");
        println!("Index len: {:#?}", sfasta.index_len());
        assert!(sfasta.index_len() == Ok(10));

        let _output = &sfasta.find("test3").unwrap().unwrap();

        sfasta.find("test").unwrap().unwrap();
        sfasta.find("test2").unwrap().unwrap();
        let out = sfasta.find("test3").unwrap().unwrap();
        println!("{:#?}", out);
        sfasta.find("test4").unwrap().unwrap();
        sfasta.find("test5").unwrap().unwrap();
    }
}
