//! Structs to open and work with SFASTA file format
//!
//! This module contains the main methods of reading SFASTA files,
//! including iterators to iterate over contained sequences.

// dev NOTES
// Only first word of header is returned now
// Sometimes the next sequence is put out without a newline
// example:
// NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNTTGATT
// AAACTAAATCTGGAATAATAATTTGTCCTTATAATAAATGGGAAGATCTTAATAAAGATC
// AGAACTAAAAAGTCCGTAAAATAAAGATCACTTCGATGATGATACAATATAAAAAAGAAA>Chr2
// TTCTTAAATCTTATAATCTTGTAAGACGTAATATAATGCTTTGAGCGtctctctctctct
// ctctctctctctgctctgtctcATTCCCAACTCGACGATGCGTCCCTTTCGTAGCGAACA
// GAGCCACCAACGTCCGACGCGACTAACTAACGACTCCGACAAGTCGATAAGGTCATAGTT
// ATTTTTATTGTTGATTAGTTGGTAAGCGAAAGAAAGTCCGTATACGCGCGTTGTTTCGGT

use std::{
    borrow::BorrowMut,
    io::{BufRead, Read, Seek, SeekFrom},
    sync::Arc,
    time::Instant,
};

#[cfg(feature = "async")]
use tokio::{
    fs::File, io::BufReader, sync::Mutex, sync::OwnedMutexGuard, sync::RwLock,
};

#[cfg(not(feature = "async"))]
use std::sync::RwLock;

#[cfg(feature = "async")]
mod async_deps
{
    pub(crate) use crate::parser::async_parser::{
        bincode_decode_from_buffer_async,
        bincode_decode_from_buffer_async_with_size_hint,
    };
    pub use async_stream::stream;
    pub use libfilehandlemanager::AsyncFileHandleManager;
    pub use libfractaltree::FractalTreeDiskAsync;
    pub use tokio_stream::{Stream, StreamExt};
}

#[cfg(feature = "async")]
use async_deps::*;

use crate::datatypes::*;
use libfractaltree::FractalTreeDisk;

use bumpalo::Bump;
use bytes::Bytes;
use xxhash_rust::xxh3::xxh3_64;

#[cfg(unix)]
use std::os::fd::AsRawFd;

#[cfg(not(feature = "async"))]
/// Main Sfasta struct
pub struct Sfasta
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

    pub buf: Option<Arc<RwLock<Box<dyn ReadAndSeek + Send + Sync>>>>,
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
pub struct Sfasta
{
    pub version: u64, /* I'm going to regret this, but
                       * 18,446,744,073,709,551,615 versions should be
                       * enough for anybody.
                       *
                       * todo u128 is stable now
                       */
    pub directory: Directory,
    pub metadata: Option<Metadata>,
    pub index: Option<Arc<FractalTreeDiskAsync<u32, u32>>>,
    pub buf: Option<Arc<RwLock<Box<dyn ReadAndSeek + Send + Sync>>>>,
    pub sequences: Option<Arc<BytesBlockStore>>,
    pub seqlocs: Option<Arc<SeqLocsStore>>,
    pub headers: Option<Arc<StringBlockStore>>,
    pub ids: Option<Arc<StringBlockStore>>,
    pub masking: Option<Arc<Masking>>,
    pub scores: Option<Arc<BytesBlockStore>>,
    pub file: Option<String>,

    // For async mode we have shared file handles
    pub file_handles: Arc<AsyncFileHandleManager>,
}

impl Default for Sfasta
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

impl Sfasta
{
    /// Right now ignores scores, but add that in soon...
    // todo channels would be better and let things work in parallel in the
    // background but good enough for now
    #[cfg(feature = "async")]
    pub fn stream(self: Arc<Self>) -> impl Stream<Item = Sequence>
    {
        let sfasta = Arc::clone(&self);

        let gen = stream! {
            // Get the generators
            let seqlocs = tokio::spawn(Arc::clone(&sfasta.seqlocs.as_ref().unwrap()).stream());

            let seqs = tokio::spawn( {
                BytesBlockStoreBlockReader::new(
                    Arc::clone(&sfasta.sequences.as_ref().unwrap()),
                    Arc::clone(&sfasta.file_handles),
            )});

            let ids = tokio::spawn( {
                StringBlockStoreSeqLocReader::new(
                    Arc::clone(&sfasta.ids.as_ref().unwrap()),
                    Arc::clone(&sfasta.file_handles),
            )});

            let headers = tokio::spawn( {
                StringBlockStoreSeqLocReader::new(
                    Arc::clone(&sfasta.headers.as_ref().unwrap()),
                    Arc::clone(&sfasta.file_handles),
            )});

            let masking = if sfasta.masking.is_some() {
                tokio::spawn( {
                    MaskingBlockReader::new(
                        Arc::clone(&sfasta.masking.as_ref().unwrap()),
                        Arc::clone(&sfasta.file_handles),
                )})
            } else {
                tokio::spawn( MaskingBlockReader::dummy() )
            };

            // let seqs = tokio::spawn(Arc::clone(&sfasta.sequences.as_ref().unwrap()).stream(Arc::clone(&fhm)));
            // let ids = tokio::spawn(Arc::clone(&sfasta.ids.as_ref().unwrap()).stream(Arc::clone(&fhm)));
            // let headers = tokio::spawn(Arc::clone(&sfasta.headers.as_ref().unwrap()).stream(Arc::clone(&fhm)));

            // let masking = Arc::clone(&sfasta.masking.as_ref().unwrap()).stream(Arc::clone(&fhm));

            // todo scores
            // todo signals
            // todo mods
            // todo flags

            let seqlocs = seqlocs.await.unwrap();
            let seqs = seqs.await.unwrap();
            let ids = ids.await.unwrap();
            let headers = headers.await.unwrap();
            let masking = masking.await.unwrap();

            tokio::pin!(seqlocs);
            tokio::pin!(seqs);
            tokio::pin!(ids);
            tokio::pin!(headers);
            tokio::pin!(masking);

            loop {
                let seqloc = match seqlocs.next().await {
                    Some(s) => s,
                    None => break,
                };

                // Get the sequence

                let seqloc = Arc::new(seqloc);

                let seq = seqs.next(seqloc.1.get_sequence());
                let id = ids.next(seqloc.1.get_ids());
                let header = headers.next(seqloc.1.get_headers());

                let mut seq = seq.await.unwrap();

                if sfasta.masking.is_some() {
                    let mask = masking.next(seqloc.1.get_masking()).await;

                    if let Some(mask) = mask {
                        mask_sequence(&mut seq, mask);
                    }
                }

                let id = id.await;
                let header = header.await;

                let sequence = Sequence {
                    sequence: Some(seq.freeze()),
                    id,
                    header,
                    scores: None,
                    offset: 0,
                };

                yield sequence
            }
        };

        gen
    }

    pub fn conversion(mut self) -> Self
    {
        self.metadata = Some(Metadata::default());
        self
    }

    #[cfg(not(feature = "async"))]
    /// Use for after cloning(primarily for multiple threads), give
    /// the object a new read buffer
    pub fn with_buffer(
        mut self,
        buf: impl Read + Seek + Send + Sync + BufRead + 'static,
    ) -> Self
    {
        let buf = Box::new(buf);
        self.buf = Some(Arc::new(RwLock::new(buf)));
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
            Some(self.get_id(matches.get_ids()).unwrap())
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
    // #[tracing::instrument(skip(self))]
    pub async fn get_sequence_by_id(
        self: Arc<Self>,
        id: &str,
    ) -> Result<Option<Sequence>, &'static str>
    {
        let matches = self.find(id).await.expect("Unable to find entry");
        if matches.is_none() {
            return Ok(None);
        }

        let matches = matches.unwrap().clone();

        let id = if matches.ids > 0 {
            Some(self.get_id(matches.get_ids()).await.unwrap())
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
        let start = Instant::now();

        let seqloc = match self.get_seqloc(idx) {
            Ok(Some(s)) => s,
            Ok(None) => return Ok(None),
            Err(e) => return Err(e),
        }
        .clone();

        assert!(seqloc.sequence > 0);

        let duration = start.elapsed();
        log::trace!("get_sequence_by_index took {:?}", duration);

        self.get_sequence_by_seqloc(&seqloc)
    }

    #[cfg(feature = "async")]
    #[tracing::instrument(skip(self))]
    pub async fn get_sequence_by_index(
        &self,
        idx: usize,
    ) -> Result<Option<Sequence>, &'static str>
    {
        let seqloc = match self.get_seqloc(idx).await {
            Ok(Some(s)) => s,
            Ok(None) => return Ok(None),
            Err(e) => return Err(e),
        };

        assert!(seqloc.sequence > 0);

        self.get_sequence_by_seqloc(seqloc).await
    }

    #[cfg(not(feature = "async"))]
    pub fn get_sequence_by_seqloc(
        &mut self,
        seqloc: &SeqLoc,
    ) -> Result<Option<Sequence>, &'static str>
    {
        let id = if seqloc.ids > 0 {
            Some(self.get_id(seqloc.get_ids()).unwrap()) // todo
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
    pub async fn get_sequence_metadata(
        &self,
        id: &str,
    ) -> Result<Option<SequenceMetadata>, &'static str>
    {
        let seqloc = match self.find(id).await {
            Ok(Some(s)) => s,
            Ok(None) => return Ok(None),
            Err(e) => return Err(e),
        };

        Ok(Some(self.seqloc_metadata(&seqloc).await))
    }

    #[cfg(feature = "async")]
    pub async fn seqloc_metadata(&self, seqloc: &SeqLoc) -> SequenceMetadata
    {
        let id = if seqloc.ids > 0 {
            Some(
                std::str::from_utf8(
                    &self.get_id(seqloc.get_ids()).await.unwrap(),
                )
                .unwrap()
                .to_string(),
            )
        } else {
            None
        };

        let header = seqloc.headers > 0;
        let masking = seqloc.masking > 0;
        let scores = seqloc.scores > 0;
        let length: u64 = seqloc
            .get_sequence()
            .iter()
            .map(|x| x.len as u64)
            .sum::<u64>();

        SequenceMetadata {
            id,
            header,
            masking,
            length,
            scores,
        }
    }

    /// Faster version, non-async, no ID decompression
    pub fn seqloc_metadata_no_id(&self, seqloc: &SeqLoc) -> SequenceMetadata
    {
        let header = seqloc.headers > 0;
        let masking = seqloc.masking > 0;
        let scores = seqloc.scores > 0;
        let length: u64 = seqloc
            .get_sequence()
            .iter()
            .map(|x| x.len as u64)
            .sum::<u64>();

        SequenceMetadata {
            id: None,
            header,
            masking,
            length,
            scores,
        }
    }

    #[cfg(feature = "async")]
    #[tracing::instrument(skip(self))]
    pub async fn get_sequence_by_seqloc(
        &self,
        seqloc: SeqLoc,
    ) -> Result<Option<Sequence>, &'static str>
    {
        let seqloc = Arc::new(seqloc);

        let id = if seqloc.ids > 0 {
            let mut buf = self.file_handles.get_filehandle().await;
            let ids = std::sync::Arc::clone(&self.ids.as_ref().unwrap());
            let seqlocs = Arc::clone(&seqloc);

            // let locs = seqloc.get_ids().to_vec();
            Some(tokio::spawn(async move {
                ids.get(&mut buf, seqlocs.get_ids()).await
            }))
        } else {
            None
        };

        let header = if seqloc.headers > 0 {
            let mut buf = self.file_handles.get_filehandle().await;
            let headers =
                std::sync::Arc::clone(&self.headers.as_ref().unwrap());
            let seqlocs = Arc::clone(&seqloc);

            Some(tokio::spawn(async move {
                headers.get(&mut buf, seqlocs.get_headers()).await
            }))
        } else {
            None
        };

        let sequence = if seqloc.sequence > 0 {
            let seqloc = Arc::clone(&seqloc);
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
            Some(x) => Some(x.await.unwrap()),
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
    #[tracing::instrument(skip(self))]
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
    #[tracing::instrument(skip(self))]
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
    ) -> Result<Bytes, &'static str>
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

        Ok(Bytes::from(seq))
    }

    #[cfg(feature = "async")]
    // TODO: Should return Result<Option<Sequence>, &str>
    // TODO: Should actually be what get_sequence_by_seqloc is!
    /// Gets the sequence specified with seqloc, and applies masking
    /// specified with maskingloc.. To have no masking just pass a
    /// blank slice.
    #[tracing::instrument(skip(self))]
    pub async fn get_sequence(
        &self,
        sequencelocs: &[Loc],
        maskinglocs: &[Loc],
    ) -> Result<Bytes, &'static str>
    {
        let start = Instant::now();
        let mut seq = Vec::with_capacity(
            sequencelocs.iter().map(|i| i.len).sum::<u32>() as usize,
        );

        let mut results = Vec::with_capacity(sequencelocs.len());

        let mask = if !maskinglocs.is_empty() && self.masking.is_some() {
            let masking = Arc::clone(self.masking.as_ref().unwrap());
            let mut buf = self.file_handles.get_filehandle().await;
            let maskinglocs_c = maskinglocs.to_vec();
            Some(tokio::spawn(async move {
                masking.get_mask(&mut buf, &maskinglocs_c).await
            }))
        } else {
            None
        };

        for l in sequencelocs.iter() {
            let fhm = Arc::clone(&self.file_handles);
            let sequences = Arc::clone(self.sequences.as_ref().unwrap());
            let l = l.clone();
            let jh = tokio::spawn(async move {
                let mut buf = fhm.get_filehandle().await;

                match sequences.get_block(&mut buf, l.block).await {
                    DataOrLater::Data(data) => data,
                    DataOrLater::Later(data) => data.await.unwrap(),
                }
            });

            results.push(jh);
        }

        for (i, r) in results.into_iter().enumerate() {
            let seqblock = r.await.unwrap();

            let l = &sequencelocs[i];
            seq.extend_from_slice(
                &seqblock[l.start as usize..(l.start + l.len) as usize],
            );
        }

        if !maskinglocs.is_empty() && self.masking.is_some() && mask.is_some() {
            mask_sequence(&mut seq, mask.unwrap().await.unwrap());
        }

        let duration = start.elapsed();
        log::trace!(
            "Get sequence took {:?} starting with block {}",
            duration,
            sequencelocs[0].block
        );

        Ok(Bytes::from(seq))
    }

    #[cfg(not(feature = "async"))]
    pub fn get_sequence_nocache(
        &mut self,
        seqloc: &SeqLoc,
    ) -> Result<Bytes, &'static str>
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

        Ok(Bytes::from(seq))
    }

    #[cfg(feature = "async")]
    #[tracing::instrument(skip(self))]
    pub async fn get_sequence_nocache(
        &self,
        seqloc: &SeqLoc,
    ) -> Result<Bytes, &'static str>
    {
        let mut seq: Vec<u8> = Vec::with_capacity(seqloc.seq_len());

        assert!(seqloc.sequence > 0);

        let mut buf = self.file_handles.get_filehandle().await;
        let locs = seqloc.get_sequence();

        let mask = if seqloc.masking > 0 && self.masking.is_some() {
            let masking = Arc::clone(self.masking.as_ref().unwrap());
            let mut buf = self.file_handles.get_filehandle().await;
            let maskinglocs_c = seqloc.get_masking().to_vec();
            Some(tokio::spawn(async move {
                masking.get_mask(&mut buf, &maskinglocs_c).await
            }))
        } else {
            None
        };

        // todo send all of these off at once, otherwise it's pretty close to
        // linear...
        for l in locs.iter() {
            let seqblock = match self
                .sequences
                .as_ref()
                .unwrap()
                .get_block(&mut buf, l.block)
                .await
            {
                DataOrLater::Data(data) => data,
                DataOrLater::Later(data) => data.await.unwrap(),
            };

            seq.extend_from_slice(
                &seqblock[l.start as usize..(l.start + l.len) as usize],
            );
        }

        if seqloc.masking > 0 && self.masking.is_some() && mask.is_some() {
            mask_sequence(&mut seq, mask.unwrap().await.unwrap());
        }

        Ok(Bytes::from(seq))
    }

    #[cfg(not(feature = "async"))]
    // No masking is possible here...
    pub fn get_sequence_by_locs(
        &mut self,
        locs: &[Loc],
    ) -> Result<Bytes, &'static str>
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

        Ok(Bytes::from(seq))
    }

    #[cfg(feature = "async")]
    #[tracing::instrument(skip(self))]
    pub async fn get_sequence_by_locs(
        &self,
        locs: &[Loc],
    ) -> Result<Bytes, &'static str>
    {
        let mut seq = Vec::with_capacity(
            locs.iter().map(|i| i.len).sum::<u32>() as usize,
        );

        let mut buf = self.file_handles.get_filehandle().await;

        // Once stabilized, use write_all_vectored
        for l in locs.iter().map(|x| x) {
            let seqblock = match self
                .sequences
                .as_ref()
                .unwrap()
                .get_block(&mut buf, l.block)
                .await
            {
                DataOrLater::Data(data) => data,
                DataOrLater::Later(data) => data.await.unwrap(),
            };

            seq.extend_from_slice(
                &seqblock[l.start as usize..(l.start + l.len) as usize],
            );
        }

        Ok(Bytes::from(seq))
    }

    #[cfg(not(feature = "async"))]
    // No masking is possible here...
    pub fn get_sequence_by_locs_nocache(
        &mut self,
        locs: &[Loc],
    ) -> Result<Bytes, &'static str>
    {
        let mut seq: Vec<u8> = Vec::with_capacity(1024);

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

        Ok(Bytes::from(seq))
    }

    #[cfg(feature = "async")]
    // No masking is possible here...
    #[tracing::instrument(skip(self))]
    pub async fn get_sequence_by_locs_nocache(
        &self,
        locs: &[Loc],
    ) -> Result<Bytes, &'static str>
    {
        let mut seq = Vec::with_capacity(
            locs.iter().map(|i| i.len).sum::<u32>() as usize,
        );

        let mut buf = self.file_handles.get_filehandle().await;

        for l in locs.iter() {
            let block = match self
                .sequences
                .as_ref()
                .unwrap()
                .get_block(&mut buf, l.block)
                .await
            {
                DataOrLater::Data(data) => data,
                DataOrLater::Later(data) => data.await.unwrap(),
            };

            seq.extend_from_slice(
                &block[l.start as usize..(l.start + l.len) as usize],
            );
        }

        Ok(Bytes::from(seq))
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
    #[tracing::instrument(skip(self))]
    pub async fn find(&self, x: &str) -> Result<Option<SeqLoc>, &'static str>
    {
        assert!(self.index.is_some(), "Sfasta index not present");

        let key = xxh3_64(x.as_bytes());

        let idx = self.index.as_ref().unwrap();

        let found = idx.search(&(key as u32)).await;

        if found.is_none() {
            return Ok(None);
        }

        let seqlocs = self.seqlocs.as_ref().unwrap();

        // todo Allow returning multiple if there are multiple matches...
        seqlocs.get_seqloc(found.unwrap()).await
    }

    #[cfg(not(feature = "async"))]
    pub fn get_header(&mut self, locs: &[Loc]) -> Result<Bytes, &'static str>
    {
        let headers = self.headers.as_mut().unwrap();
        let mut buf = &mut *self.buf.as_ref().unwrap().write().unwrap();

        Ok(headers.get(&mut buf, &locs))
    }

    #[cfg(feature = "async")]
    #[tracing::instrument(skip(self))]
    pub async fn get_header(&self, locs: &[Loc])
        -> Result<Bytes, &'static str>
    {
        let headers = self.headers.as_ref().unwrap();
        let mut buf = self.file_handles.get_filehandle().await;

        Ok(headers.get(&mut buf, &locs).await)
    }

    #[cfg(not(feature = "async"))]
    pub fn get_id(&mut self, locs: &[Loc]) -> Result<Bytes, &'static str>
    {
        let mut buf = &mut *self.buf.as_ref().unwrap().write().unwrap();
        let ids = self.ids.as_mut().unwrap();
        Ok(ids.get(&mut buf, &locs))
    }

    #[cfg(feature = "async")]
    #[tracing::instrument(skip(self))]
    pub async fn get_id(&self, locs: &[Loc]) -> Result<Bytes, &'static str>
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
    #[tracing::instrument(skip(self))]
    pub async fn len(&self) -> usize
    {
        let mut buf = self.file_handles.get_filehandle().await;
        self.seqlocs.as_ref().unwrap().len(&mut buf).await
    }

    // Get length from SeqLoc (only for sequence)
    pub fn seqloc_len(&mut self, seqloc: &SeqLoc) -> usize
    {
        seqloc.seq_len()
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
    #[tracing::instrument(skip(self))]
    pub async fn get_seqloc(
        &self,
        i: usize,
    ) -> Result<Option<SeqLoc>, &'static str>
    {
        // assert!(i < self.len().await, "Index out of bounds");
        assert!(i < std::u32::MAX as usize, "Index out of bounds");

        self.seqlocs.as_ref().unwrap().get_seqloc(i as u32).await
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
    #[tracing::instrument(skip(self))]
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
    #[tracing::instrument(skip(self))]
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
    #[tracing::instrument(skip(self))]
    pub async fn index_load(&self) -> Result<(), &'static str>
    {
        Arc::clone(self.index.as_ref().unwrap()).load_tree().await
    }
}

/// Sfasta is designed for random access, but sometimes it is still
/// useful to iterate through the file in a linear fashion...
///
/// Because the access patterns are so different, this is a
/// specialized implementation for speed.
// todo make non-async version
#[cfg(feature = "async")]
pub struct SfastaIterator
{
    sfasta: Arc<Sfasta>,
    current: usize,
    runtime: tokio::runtime::Runtime,

    next_seqloc: tokio::task::JoinHandle<Result<Option<SeqLoc>, &'static str>>,

    seq_blocks_cache: Vec<(u32, Bytes)>,
    masking_blocks_cache: Vec<(u32, Bytes)>,
    ids_blocks_cache: Vec<(u32, Bytes)>,
    headers_blocks_cache: Vec<(u32, Bytes)>,
    // scores_blocks_cache: Vec<Bytes>,
    // signals_blocks_cache: Vec<Bytes>,
    // seq_cache: Bump,
    // masking_cache: Bump,
    // ids_cache: Bump,
    // headers_cache: Bump,
    // scores_cache: Bump,
    // signals_cache: Bump,
}

#[cfg(feature = "async")]
impl SfastaIterator
{
    pub fn new(sfasta: Sfasta) -> Self
    {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();

        let sfasta = Arc::new(sfasta);
        let sfasta_c = Arc::clone(&sfasta);
        let next_seqloc: tokio::task::JoinHandle<Result<Option<SeqLoc>, &str>> =
            runtime.spawn(async move { sfasta_c.get_seqloc(0).await });

        SfastaIterator {
            sfasta,
            current: 0,

            next_seqloc,

            seq_blocks_cache: Vec::new(),
            masking_blocks_cache: Vec::new(),
            ids_blocks_cache: Vec::new(),
            headers_blocks_cache: Vec::new(),

            // seq_cache: Bump::with_capacity(512 * 1024),
            // masking_cache: Bump::with_capacity(512 * 1024),
            // ids_cache: Bump::with_capacity(512 * 1024),
            // headers_cache: Bump::with_capacity(512 * 1024),
            runtime,
        }
    }
}

#[cfg(feature = "async")]
impl Iterator for SfastaIterator
{
    type Item = Result<Sequence, &'static str>;

    fn next(&mut self) -> Option<Self::Item>
    {
        let sfasta_c = Arc::clone(&self.sfasta);
        let idx = self.current + 1;
        let mut next_seqloc: tokio::task::JoinHandle<
            Result<Option<SeqLoc>, &str>,
        > = self
            .runtime
            .spawn(async move { sfasta_c.get_seqloc(idx).await });

        std::mem::swap(&mut self.next_seqloc, &mut next_seqloc);

        let seqloc = self.runtime.block_on(next_seqloc).unwrap().unwrap();
        let seqloc = match seqloc {
            Some(s) => s,
            None => return None,
        };

        // Get blocks for the sequence if not already loaded
        let seq_locs = seqloc.get_sequence();
        let seq_blocks = seq_locs.iter().map(|l| l.block).collect::<Vec<u32>>();

        // Remove any blocks that aren't needed from the cache
        self.seq_blocks_cache
            .retain(|(block, _)| seq_blocks.contains(block));

        // Load any blocks that aren't in the cache for sequences
        for block_idx in seq_blocks.iter() {
            if self
                .seq_blocks_cache
                .iter()
                .find(|(b, _)| b == block_idx)
                .is_none()
            {
                let sfasta_c = Arc::clone(&self.sfasta);
                let idx = *block_idx as u32;
                let block = self.runtime.block_on(async move {
                    let mut buf = sfasta_c.file_handles.get_filehandle().await;
                    sfasta_c
                        .sequences
                        .as_ref()
                        .unwrap()
                        .get_block(&mut buf, idx)
                        .await
                });

                let block = match block {
                    DataOrLater::Data(data) => data,
                    DataOrLater::Later(data) => {
                        self.runtime.block_on(data).unwrap()
                    }
                };

                self.seq_blocks_cache.push((*block_idx, block));
            }
        }

        let mut seq = Vec::with_capacity(seqloc.seq_len());

        // Build the sequence
        for l in seq_locs.iter() {
            let block = self
                .seq_blocks_cache
                .iter()
                .find(|(b, _)| b == &l.block)
                .unwrap()
                .1
                .clone();
            seq.extend_from_slice(
                &block[l.start as usize..(l.start + l.len) as usize],
            );
        }

        // Just testing right now, so no need for any of the other stuff...
        return Some(Ok(Sequence {
            sequence: Some(Bytes::from(seq)),
            id: None,
            header: None,
            scores: None,
            offset: 0,
        }));
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
    use crate::parser::open_with_buffer;

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
        // let mut sfasta = SfastaParser::open_from_buffer(out_buf).unwrap();
        let mut sfasta = open_with_buffer(out_buf).expect("Unable to open");
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

        // let mut sfasta = SfastaParser::open_from_buffer(out_buf).unwrap();
        let mut sfasta = open_with_buffer(out_buf).expect("Unable to open");

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
