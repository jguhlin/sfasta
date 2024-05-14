//! Structs to open and work with SFASTA file format
//!
//! This module contains the main methods of reading SFASTA files,
//! including iterators to iterate over contained sequences.

use std::{
    io::{BufRead, Read, Seek, SeekFrom},
    sync::Arc,
};

#[cfg(feature = "async")]
use tokio::{
    io::BufReader,
    fs::File,
    sync::RwLock
};

#[cfg(not(feature = "async"))]
use std::sync::RwLock;

use libfractaltree::{FractalTreeDisk, FractalTreeDiskAsync};

use rand::prelude::*;
use rand_chacha::ChaCha20Rng;
use xxhash_rust::xxh3::xxh3_64;

#[cfg(feature = "async")]
use tokio::task::JoinSet;

use crate::datatypes::*;

#[cfg(not(feature = "async"))]
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
}

#[cfg(feature = "async")]
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
    pub index: Option<FractalTreeDiskAsync<u32, u32>>,
    pub buf: Option<RwLock<Box<dyn ReadAndSeek + Send + Sync + 'sfa>>>,
    pub sequences: Option<Arc<BytesBlockStore>>,
    pub seqlocs: Option<Arc<SeqLocsStore>>,
    pub headers: Option<Arc<StringBlockStore>>,
    pub ids: Option<Arc<StringBlockStore>>,
    pub masking: Option<Arc<Masking>>,
    pub scores: Option<Arc<BytesBlockStore>>,
    pub file: Option<String>,

    // For async mode we have shared file handles
    pub file_handles: Arc<AsyncFileHandleManager>    
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
            file_handles: Arc::new(AsyncFileHandleManager::default()),
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

    #[cfg(not(feature = "async"))]
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

    #[cfg(feature = "async")]
    /// Get a Sequence object by ID.
    /// Convenience function. Not optimized for speed. If you don't
    /// need the header, scores, or masking, it's better to call
    /// more performant functions.
    pub async fn get_sequence_by_id(
        &self,
        id: &str,
    ) -> Result<Option<Sequence>, &str>
    {
        let matches = self.find(id).await.expect("Unable to find entry");
        if matches.is_none() {
            return Ok(None);
        }

        let matches = matches.unwrap().clone();

        let id = if matches.ids > 0 {
            Some(self.get_id(matches.get_ids()).await.unwrap().into_bytes())
        // todo
        } else {
            None
        };

        let header = if matches.headers > 0 {
            Some(self.get_header(matches.get_headers()).await.unwrap().into())
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
                .await
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

    #[cfg(not(feature = "async"))]
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

    #[cfg(feature = "async")]
    pub async fn get_sequence_by_index(
        &self,
        idx: usize,
    ) -> Result<Option<Sequence>, &'static str>
    {
        let seqloc = match self.get_seqloc(idx).await {
            Ok(Some(s)) => s,
            Ok(None) => return Ok(None),
            Err(e) => return Err(e),
        }
        .clone();

        assert!(seqloc.sequence > 0);

        self.get_sequence_by_seqloc(&seqloc).await
    }

    #[cfg(not(feature = "async"))]
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

    #[cfg(feature = "async")]
    pub async fn get_sequence_by_seqloc(
        &self,
        seqloc: &SeqLoc,
    ) -> Result<Option<Sequence>, &'static str>
    {

        let id = if seqloc.ids > 0 {
            let mut buf = self.file_handles.get_filehandle().await;
            let ids = std::sync::Arc::clone(&self.ids.as_ref().unwrap());
            let locs = seqloc.get_ids().to_vec();
            Some(tokio::spawn(async move {
                ids.get(&mut buf, &locs).await
            }))
        } else {
            None
        };

        let header = if seqloc.headers > 0 {
            let mut buf = self.file_handles.get_filehandle().await;
            let headers = std::sync::Arc::clone(&self.headers.as_ref().unwrap());
            let locs = seqloc.get_headers().to_vec();
            Some(tokio::spawn(async move {
                headers.get(&mut buf, &locs).await
            }))
        } else {
            None
        };

        let sequence = if seqloc.sequence > 0 {
            Some(
                self.get_sequence(seqloc.get_sequence(), seqloc.get_masking())
                    .await
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

        let id = match id {
            Some(x) => Some(x.await.unwrap().into_bytes()),
            None => None,
        };

        let header = match header {
            Some(x) => Some(x.await.unwrap().into()),
            None => None,
        };

        Ok(Some(Sequence {
            sequence,
            id,
            header,
            scores: None,
            offset: 0,
        }))
    }

    #[cfg(not(feature = "async"))]
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

    #[cfg(feature = "async")]
    pub async fn get_sequence_only_by_seqloc(
        &self,
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
                    .await
                    .unwrap(),
                )
            } else {
                Some(self.get_sequence_nocache(seqloc).await.unwrap())
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

    #[cfg(not(feature = "async"))]
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

    #[cfg(feature = "async")]
    pub async fn get_sequence_only_by_locs(
        &self,
        locs: &[Loc],
        cache: bool,
    ) -> Result<Option<Sequence>, &'static str>
    {
        let sequence = if cache {
            Some(self.get_sequence_by_locs(locs).await.unwrap())
        } else {
            Some(self.get_sequence_by_locs_nocache(locs).await.unwrap())
        };

        Ok(Some(Sequence {
            sequence,
            id: None,
            header: None,
            scores: None,
            offset: 0,
        }))
    }

    #[cfg(not(feature = "async"))]
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

    #[cfg(feature = "async")]
    // TODO: Should return Result<Option<Sequence>, &str>
    // TODO: Should actually be what get_sequence_by_seqloc is!
    /// Gets the sequence specified with seqloc, and applies masking
    /// specified with maskingloc.. To have no masking just pass a
    /// blank slice.
    pub async fn get_sequence(
        &self,
        seqloc: &[Loc],
        maskingloc: &[Loc],
    ) -> Result<Vec<u8>, &'static str>
    {
        let mut seq: Vec<u8> = Vec::new();

        let mut results = Vec::with_capacity(seqloc.len());

        for l in seqloc.into_iter() {
            let fhm = Arc::clone(&self.file_handles);
            let sequences = Arc::clone(self.sequences.as_ref().unwrap());
            let l = l.clone();
            let jh = tokio::spawn(async move {
                let mut buf = fhm.get_filehandle().await;
                let seqblock = sequences
                .get_block(&mut buf, l.block)
                .await;

                seqblock[l.start as usize..(l.start + l.len) as usize].to_vec()
            });

            results.push(jh);
        }

        for r in results {
            seq.extend_from_slice(&r.await.unwrap());
        }

        /*
        let mut buf = self.get_filehandle().await;

        // todo send all of these off at once, otherwise it's pretty close to
        // linear...
        for l in seqloc.iter() {
            let seqblock = self
                .sequences
                .as_ref()
                .unwrap()
                .get_block(&mut buf, l.block)
                .await;
            seq.extend_from_slice(
                &seqblock[l.start as usize..(l.start + l.len) as usize],
            );
        } 
        */

        let mut buf = self.file_handles.get_filehandle().await;
        if !maskingloc.is_empty() && self.masking.is_some() {
            let masking = self.masking.as_ref().unwrap();
            masking.mask_sequence(&mut buf, maskingloc, &mut seq).await;
        }

        Ok(seq)
    }

    #[cfg(not(feature = "async"))]
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

    #[cfg(feature = "async")]
    pub async fn get_sequence_nocache(
        &self,
        seqloc: &SeqLoc,
    ) -> Result<Vec<u8>, &'static str>
    {
        let mut seq: Vec<u8> = Vec::new();

        assert!(seqloc.sequence > 0);

        let mut buf = self.file_handles.get_filehandle().await;
        let locs = seqloc.get_sequence();

        // todo send all of these off at once, otherwise it's pretty close to
        // linear...
        for l in locs.iter() {
            let seqblock = self
                .sequences
                .as_ref()
                .unwrap()
                .get_block(&mut buf, l.block)
                .await;
            seq.extend_from_slice(
                &seqblock[l.start as usize..(l.start + l.len) as usize],
            );
        }

        if seqloc.masking > 0 && self.masking.is_some() {
            let masking = self.masking.as_ref().unwrap();
            let locs = seqloc.get_masking();
            masking.mask_sequence(&mut buf, &locs, &mut seq).await;
        }

        Ok(seq)
    }

    #[cfg(not(feature = "async"))]
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

    #[cfg(feature = "async")]
    pub async fn get_sequence_by_locs(
        &self,
        locs: &[Loc],
    ) -> Result<Vec<u8>, &'static str>
    {
        let mut seq: Vec<u8> = Vec::with_capacity(1024);

        let mut buf = self.file_handles.get_filehandle().await;

        // Once stabilized, use write_all_vectored
        for l in locs.iter().map(|x| x) {
            let seqblock = self
                .sequences
                .as_ref()
                .unwrap()
                .get_block(&mut buf, l.block)
                .await;
            seq.extend_from_slice(
                &seqblock[l.start as usize..(l.start + l.len) as usize],
            );
        }

        Ok(seq)
    }

    #[cfg(not(feature = "async"))]
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

    #[cfg(feature = "async")]
    // No masking is possible here...
    pub async fn get_sequence_by_locs_nocache(
        &self,
        locs: &[Loc],
    ) -> Result<Vec<u8>, &'static str>
    {
        let mut seq: Vec<u8> = Vec::with_capacity(256);

        let mut buf = self.file_handles.get_filehandle().await;

        for l in locs.iter() {
            let block = self
                .sequences
                .as_ref()
                .unwrap()
                .get_block(&mut buf, l.block)
                .await;

            seq.extend_from_slice(
                &block[l.start as usize..(l.start + l.len) as usize],
            );
        }

        Ok(seq)
    }

    #[cfg(not(feature = "async"))]
    pub fn find(&mut self, x: &str) -> Result<Option<SeqLoc>, &str>
    {
        assert!(self.index.is_some(), "Sfasta index not present");

        let key = xxh3_64(x.as_bytes());

        let idx = self.index.as_mut().unwrap();

        let mut buf = &mut *self.buf.as_ref().unwrap().write().unwrap();

        let found = idx.search(&mut buf, &(key as u32));
        let seqlocs = self.seqlocs.as_mut().unwrap();

        if found.is_none() {
            return Ok(None);
        }

        // TODO: Allow returning multiple if there are multiple matches...
        log::debug!("Getting seqloc");
        seqlocs.get_seqloc(&mut buf, found.unwrap())
    }

    #[cfg(feature = "async")]
    pub async fn find(&self, x: &str) -> Result<Option<SeqLoc>, &'static str>
    {
        assert!(self.index.is_some(), "Sfasta index not present");

        let key = xxh3_64(x.as_bytes());

        let idx = self.index.as_ref().unwrap();

        let mut buf = self.file_handles.get_filehandle().await;

        let found = idx.search(&mut buf, &(key as u32)).await;
        let seqlocs = self.seqlocs.as_ref().unwrap();

        if found.is_none() {
            return Ok(None);
        }

        // todo Allow returning multiple if there are multiple matches...
        seqlocs.get_seqloc(&mut buf, found.unwrap()).await
    }

    #[cfg(not(feature = "async"))]
    pub fn get_header(&mut self, locs: &[Loc]) -> Result<String, &'static str>
    {
        let headers = self.headers.as_mut().unwrap();
        let mut buf = &mut *self.buf.as_ref().unwrap().write().unwrap();

        Ok(headers.get(&mut buf, &locs))
    }

    #[cfg(feature = "async")]
    pub async fn get_header(&self, locs: &[Loc])
        -> Result<String, &'static str>
    {
        let headers = self.headers.as_ref().unwrap();
        let mut buf = self.file_handles.get_filehandle().await;

        Ok(headers.get(&mut buf, &locs).await)
    }

    #[cfg(not(feature = "async"))]
    pub fn get_id(&mut self, locs: &[Loc]) -> Result<String, &'static str>
    {
        let mut buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        let ids = self.ids.as_mut().unwrap();
        Ok(ids.get(&mut buf, &locs))
    }

    #[cfg(feature = "async")]
    pub async fn get_id(&self, locs: &[Loc]) -> Result<String, &'static str>
    {
        let mut buf = self.file_handles.get_filehandle().await;
        let ids = self.ids.as_ref().unwrap();
        Ok(ids.get(&mut buf, &locs).await)
    }

    pub fn get_id_loaded(&self, locs: &[Loc]) -> Result<String, &'static str>
    {
        let ids = self.ids.as_ref().unwrap();

        Ok(ids.get_loaded(&locs))
    }

    #[cfg(not(feature = "async"))]
    pub fn len(&mut self) -> usize
    {
        let mut buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        self.seqlocs.as_mut().unwrap().len(&mut buf)
    }

    #[cfg(feature = "async")]
    pub async fn len(&self) -> usize
    {
        let mut buf = self.file_handles.get_filehandle().await;
        self.seqlocs.as_ref().unwrap().len(&mut buf).await
    }

    // Get length from SeqLoc (only for sequence)
    pub fn seqloc_len(&mut self, seqloc: &SeqLoc) -> usize
    {
        seqloc.len()
    }

    #[cfg(not(feature = "async"))]
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

    #[cfg(feature = "async")]
    /// Get the ith seqloc in the file
    pub async fn get_seqloc(
        &self,
        i: usize,
    ) -> Result<Option<SeqLoc>, &'static str>
    {
        // assert!(i < self.len().await, "Index out of bounds");
        assert!(i < std::u32::MAX as usize, "Index out of bounds");

        let mut buf = self.file_handles.get_filehandle().await;
        self.seqlocs
            .as_ref()
            .unwrap()
            .get_seqloc(&mut buf, i as u32)
            .await
    }

    #[cfg(not(feature = "async"))]
    /// Get all seqlocs
    pub fn get_seqlocs(&mut self) -> Result<(), &'static str>
    {
        let mut buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        self.seqlocs.as_mut().unwrap().prefetch(&mut buf);

        // TODO: Fail if index is not initialized yet (prefetch does it here,
        // but still)
        self.seqlocs.as_mut().unwrap().get_all_seqlocs(&mut buf)
    }

    #[cfg(feature = "async")]
    /// Get all seqlocs
    pub async fn get_seqlocs(&self) -> Result<(), &'static str>
    {
        let mut buf = self.file_handles.get_filehandle().await;

        self.seqlocs.as_ref().unwrap().prefetch(&mut buf).await;

        self.seqlocs
            .as_ref()
            .unwrap()
            .get_all_seqlocs(&mut buf)
            .await
    }

    #[cfg(not(feature = "async"))]
    /// This is more expensive than getting it from the seqlocs
    pub fn index_len(&self) -> Result<usize, &'static str>
    {
        if self.index.is_none() {
            return Err("Index does not exist");
        }

        self.index.as_ref().unwrap().len()
    }

    #[cfg(feature = "async")]
    /// This is more expensive than getting it from the seqlocs
    pub async fn index_len(&self) -> Result<usize, &'static str>
    {
        if self.index.is_none() {
            return Err("Index does not exist");
        }

        self.index.as_ref().unwrap().len().await
    }

    #[cfg(not(feature = "async"))]
    pub fn index_load(&mut self) -> Result<(), &'static str>
    {
        let buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        self.index.as_mut().unwrap().load_tree(buf)
    }

    #[cfg(feature = "async")]
    pub async fn index_load(&self) -> Result<(), &'static str>
    {
        let mut buf = self.file_handles.get_filehandle().await;
        self.index.as_ref().unwrap().load_tree(&mut buf).await
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

#[cfg(feature = "async")]
pub struct AsyncFileHandleManager {
    pub file_handles: Option<Arc<RwLock<Vec<Arc<RwLock<BufReader<File>>>>>>>,
    pub file_name: Option<String>,
}

#[cfg(feature = "async")]
impl Default for AsyncFileHandleManager {
    fn default() -> Self {
        AsyncFileHandleManager {
            file_handles: None,
            file_name: None,
        }
    }
}

#[cfg(feature = "async")]
impl AsyncFileHandleManager {
    pub async fn get_filehandle(
        &self,
    ) -> tokio::sync::OwnedRwLockWriteGuard<tokio::io::BufReader<tokio::fs::File>>
    {
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
                // There are no available file handles, so we need to create a
                // new one and the number isn't crazy (yet)
                break;
            }

            drop(file_handles_read);

            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }

        // Otherwise, create one and add it to the list
        let file = tokio::fs::File::open(self.file_name.as_ref().unwrap())
            .await
            .unwrap();
        let file_handle = std::sync::Arc::new(tokio::sync::RwLock::new(
            tokio::io::BufReader::new(file),
        ));

        let mut write_lock = file_handles.write().await;

        write_lock.push(Arc::clone(&file_handle));

        file_handle.try_write_owned().unwrap()
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

    #[cfg(not(feature = "async"))]
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

        // let mut sfasta = SfastaParser::open_from_buffer(out_buf).unwrap();
        let mut sfasta = open_with_buffer(out_buf).expect("Unable to open");
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

    #[cfg(not(feature = "async"))]
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

    #[cfg(not(feature = "async"))]
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
