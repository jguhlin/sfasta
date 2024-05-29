//! SeqLocs handle the location pointers for sequences, masks, IDs,
//! etc... Each SeqLoc will point to a start and a length of Loc
//! objects for a given sequence
//!
//! A Loc is a pointer to a sequence, and the start of it and length,
//! which are all u32's and found in the struct Loc
//!
//! SeqLocsStore is the collection of all seqlocs
//! SeqLocs are the complete data for a specific sequence
//! Locs are the pointers to the sequence data (and strings, etc)
//!
//! SeqLocs are pointed to from the index (FractalTree<u64, (&str,
//! u64)) todo: make u32 for the start of the seq in the file?

// Stored as a row, as we will often need to decode everything

// Sequences (ids, masking, scores, header, etc...)
// are stored in blocks, and then the range of the sequence
// SeqLoc stores Locs, which are (u32, u32, u32) (block, start, len)
// as struct Loc

// TODO 5 Apr 2024 Add get_seqloc_by_index to SeqLocsStore
// Just search through them all (or if prefetched, access direct...)

use std::{
    io::{BufRead, BufWriter, Read, Seek, SeekFrom, Write},
    ops::Range,
    sync::{atomic::AtomicU32, Arc},
};

use bincode::{Decode, Encode};
use flate2::Compression;
use rand::seq;

use libcompression::*;
use libfractaltree::{FractalTreeBuild, FractalTreeDisk};

#[cfg(feature = "async")]
use async_stream::stream;

#[cfg(feature = "async")]
use tokio_stream::Stream;

#[cfg(feature = "async")]
use tokio_stream::StreamExt;

#[cfg(feature = "async")]
use libfractaltree::FractalTreeDiskAsync;

// So each SeqLoc is:
// Each seq, masking, scores, header, ids, are the number for each
// type Then the Locs are stored in a Vec<Loc>
// So masking Locs are found as locs[sequence..sequence+masking]
// Compressed version is simply Vec<u8> of all the Locs
// TODO: Base mods?
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SeqLoc
{
    pub sequence: u16,
    pub masking: u16,
    pub scores: u16,
    pub signal: u16, // FUTURE: Add Nanopore signals (and/or others)
    pub headers: u16,
    pub ids: u16,
    pub mods: u16, // FUTURE: Add mods
    pub locs: Vec<Loc>,
}

impl SeqLoc
{
    pub const fn new() -> Self
    {
        Self {
            ids: 0,
            sequence: 0,
            masking: 0,
            scores: 0,
            signal: 0,
            headers: 0,
            mods: 0,
            locs: Vec::new(),
        }
    }

    pub fn ids_pos(&self) -> Range<usize>
    {
        0..self.ids as usize
    }

    pub fn sequence_pos(&self) -> Range<usize>
    {
        self.ids as usize..(self.sequence + self.ids) as usize
    }

    pub fn masking_pos(&self) -> Range<usize>
    {
        (self.sequence + self.ids) as usize
            ..(self.sequence + self.ids + self.masking) as usize
    }

    pub fn scores_pos(&self) -> Range<usize>
    {
        (self.sequence + self.masking + self.ids) as usize
            ..(self.sequence + self.masking + self.ids + self.scores) as usize
    }

    pub fn signal_pos(&self) -> Range<usize>
    {
        (self.sequence + self.masking + self.scores + self.ids) as usize
            ..(self.sequence
                + self.masking
                + self.scores
                + self.ids
                + self.signal) as usize
    }

    pub fn headers_pos(&self) -> Range<usize>
    {
        (self.sequence + self.masking + self.scores + self.signal + self.ids)
            as usize
            ..(self.sequence
                + self.masking
                + self.scores
                + self.signal
                + self.ids
                + self.headers) as usize
    }

    pub fn mods_pos(&self) -> Range<usize>
    {
        (self.sequence
            + self.masking
            + self.scores
            + self.signal
            + self.headers
            + self.ids) as usize
            ..(self.sequence
                + self.masking
                + self.scores
                + self.signal
                + self.headers
                + self.ids
                + self.mods) as usize
    }

    pub fn get_sequence(&self) -> &[Loc]
    {
        &self.locs[self.sequence_pos()]
    }

    pub fn get_masking(&self) -> &[Loc]
    {
        &self.locs[self.masking_pos()]
    }

    pub fn get_scores(&self) -> &[Loc]
    {
        &self.locs[self.scores_pos()]
    }

    pub fn get_signal(&self) -> &[Loc]
    {
        &self.locs[self.signal_pos()]
    }

    pub fn get_headers(&self) -> &[Loc]
    {
        &self.locs[self.headers_pos()]
    }

    pub fn get_ids(&self) -> &[Loc]
    {
        &self.locs[self.ids_pos()]
    }

    pub fn add_locs(
        &mut self,
        ids: &[Loc],
        sequence: &[Loc],
        masking: &[Loc],
        scores: &[Loc],
        headers: &[Loc],
        mods: &[Loc],
        signal: &[Loc],
    )
    {
        self.sequence = sequence.len() as u16;
        self.masking = masking.len() as u16;
        self.scores = scores.len() as u16;
        self.signal = signal.len() as u16;
        self.headers = headers.len() as u16;
        self.ids = ids.len() as u16;
        self.mods = mods.len() as u16;

        self.locs.extend_from_slice(ids);
        self.locs.extend_from_slice(sequence);
        self.locs.extend_from_slice(masking);
        self.locs.extend_from_slice(scores);
        self.locs.extend_from_slice(signal);
        self.locs.extend_from_slice(headers);
        self.locs.extend_from_slice(mods);
    }

    pub fn has_headers(&self) -> bool
    {
        self.headers > 0
    }

    pub fn has_ids(&self) -> bool
    {
        self.ids > 0
    }

    pub fn has_masking(&self) -> bool
    {
        self.masking > 0
    }

    pub fn has_scores(&self) -> bool
    {
        self.scores > 0
    }

    pub fn has_signal(&self) -> bool
    {
        self.signal > 0
    }

    pub fn has_sequence(&self) -> bool
    {
        self.sequence > 0
    }

    #[allow(clippy::len_without_is_empty)]
    pub fn seq_len(&self) -> usize
    {
        self.get_sequence().iter().map(|loc| loc.len as usize).sum()
    }

    // Convert Vec of Locs to the ranges of the sequence...
    pub fn seq_location_splits(sequence: &[Loc]) -> Vec<std::ops::Range<u32>>
    {
        let mut locations = Vec::with_capacity(sequence.len());

        if sequence.is_empty() {
            return locations;
        }

        let mut start = 0;

        for loc in sequence {
            let len = loc.len;
            locations.push(start..start + len);
            start += len;
        }
        locations
    }

    // Convert range to locs to slice
    // Have to map ranges to the Vec<Locs>
    // TODO: Make generic over sequence, scores, and masking
    // TODO: Should work on staggered Locs, even though they do not
    // exist....
    pub fn seq_slice(
        &self,
        block_size: u32,
        range: std::ops::Range<u32>,
    ) -> Vec<Loc>
    {
        let mut new_locs = Vec::new();

        let locs = self.get_sequence();

        let splits = SeqLoc::seq_location_splits(&locs);

        let end = range.end - 1;

        for (i, split) in splits.iter().enumerate() {
            if split.contains(&range.start) && split.contains(&end) {
                // This loc contains the entire range
                // So for example, Loc is 1500..2000, and the range we want is
                // 20..50 (translates to 1520..1550)
                let start = range.start.saturating_sub(split.start);
                let end = range.end.saturating_sub(split.start);
                new_locs.push(locs[i].slice(start..end));
                break; // We are done if it contains the entire
                       // range...
            } else if split.contains(&range.start) {
                // Loc contains the start of the range...
                // For example, Loc is 1500..2000 (length 500 in this Loc) and
                // the range we want is 450..550 (so 1950..2000
                // from this loc, and another 100 from the next loc)
                let start = range.start.saturating_sub(split.start);
                let end = block_size; // range.end.saturating_sub(split.start);
                new_locs.push(locs[i].slice(start..end));
            } else if split.contains(&end) {
                // Loc contains the end of the range...
                // For example, Loc is 1500..2000 (length 500 in this Loc,
                // starting at 1000) and the range we want is
                // 900..1200 (so 1500..1700 from this loc)
                let start = range.start.saturating_sub(split.start);
                let end = range.end.saturating_sub(split.start);
                new_locs.push(locs[i].slice(start..end));
                break; // We are done if it contains the end of the
                       // range...
            } else if split.start > range.start && split.end < range.end {
                // Loc contains the entire range...
                // For example, Loc is 1500..2000 (length 500 in the Loc) and
                // the range we want is 450..550 (so 1500..1550
                // from this loc, and another 100 from the previous loc)
                new_locs.push(locs[i].clone());
            } else {
                // Loc does not contain the range...
                // For example, Loc is 1500..2000 (length 500 in the
                // Loc) and the range we want is 250..350 (so
                // 1750..1800 from this loc)
            }
        }

        new_locs
    }
}

impl Encode for SeqLoc
{
    fn encode<E: bincode::enc::Encoder>(
        &self,
        encoder: &mut E,
    ) -> core::result::Result<(), bincode::error::EncodeError>
    {
        let SeqLoc {
            sequence,
            masking,
            scores,
            signal,
            headers,
            ids,
            mods,
            locs,
        } = self;

        let values: [u16; 7] =
            [*ids, *sequence, *masking, *scores, *signal, *headers, *mods];

        // TODO: Is there a way to directly access the Vec<Loc> to get
        // Vec<u32>'s? Check out bytemuck (and reddit question I asked
        // about it)
        let locs = locs
            .iter()
            .map(|loc| loc.get_values())
            .flatten()
            .collect::<Vec<u32>>();

        // Subtract the lowest loc from all locs
        // todo see if this works before implementing in decode
        // saved 3% of space, without adding the min back in... so not really
        // worth it... let min = locs.iter().min().unwrap();
        // let locs = locs.iter().map(|loc| loc - min).collect::<Vec<u32>>();

        bincode::Encode::encode(&values, encoder)?;
        bincode::Encode::encode(&locs, encoder)?;
        Ok(())
    }
}

impl Decode for SeqLoc
{
    fn decode<D: bincode::de::Decoder>(
        decoder: &mut D,
    ) -> core::result::Result<Self, bincode::error::DecodeError>
    {
        let values: [u16; 7] = bincode::Decode::decode(decoder)?;

        let locs: Vec<u32> = bincode::Decode::decode(decoder)?;
        let locs = locs
            .chunks_exact(3)
            .map(|chunk| Loc::from(chunk))
            .collect::<Vec<Loc>>();

        Ok(SeqLoc {
            ids: values[0],
            sequence: values[1],
            masking: values[2],
            scores: values[3],
            signal: values[4],
            headers: values[5],
            mods: values[6],
            locs,
        })
    }
}

// TODO! Need tests...
// TODO: When data gets too large, pre-emptively compress it into
// memory (such as nt db, >200Gb).

/// Handles access to SeqLocs
pub struct SeqLocsStoreBuilder
{
    pub location: u64,
    pub tree: FractalTreeBuild<u32, SeqLoc>,
    pub compression: CompressionConfig,
    count: usize,

    create_dict: bool,
    dict_data: Vec<Vec<u8>>,
    dict_size: u64,
    dict_samples: u64,
}

impl Default for SeqLocsStoreBuilder
{
    fn default() -> Self
    {
        SeqLocsStoreBuilder {
            location: 0,
            tree: FractalTreeBuild::new(512, 8192),
            compression: CompressionConfig {
                compression_type: CompressionType::ZSTD,
                compression_level: 1,
                compression_dict: None,
            },
            count: 0,

            // none of this is implemented (because it's all in the fractal
            // tree!)
            dict_data: Vec::new(),
            dict_size: 64 * 1024,
            dict_samples: 128,
            create_dict: false,
            // todo raw and compressed sizes
        }
    }
}

impl SeqLocsStoreBuilder
{
    pub fn with_dict(mut self) -> Self
    {
        self.create_dict = true;
        self
    }

    pub fn with_dict_samples(mut self, dict_samples: u64) -> Self
    {
        self.dict_samples = dict_samples;
        self
    }

    pub fn with_dict_size(mut self, dict_size: u64) -> Self
    {
        self.dict_size = dict_size;
        self
    }

    /// Create a new SeqLocs object
    pub fn new() -> Self
    {
        SeqLocsStoreBuilder::default()
    }

    pub fn with_compression(mut self, compression: CompressionConfig) -> Self
    {
        self.compression = compression;
        self
    }

    /// Add a SeqLoc to the store
    pub fn add_to_index(&mut self, seqloc: SeqLoc) -> u32
    {
        self.tree.insert(self.count as u32, seqloc);
        self.count += 1;
        assert!(self.count < u32::MAX as usize, "Too many SeqLocs");
        self.count.saturating_sub(1) as u32
    }

    /// Set the location u64 of the SeqLocs object
    pub fn with_location(mut self, location: u64) -> Self
    {
        self.location = location;
        self
    }

    /// Write a SeqLocs object to a file (buffer), destroys the
    /// SeqLocs struct
    pub fn write_to_buffer<W>(
        mut self,
        mut out_buf: &mut W,
    ) -> Result<u64, &'static str>
    where
        W: Write + Seek,
    {
        log::trace!("Storing SeqLoc Fractal Tree");
        self.location = out_buf.stream_position().unwrap();

        let mut tree: FractalTreeDisk<u32, SeqLoc> = self.tree.into();

        let dict = if self.create_dict {
            Some(tree.create_zstd_dict())
        } else {
            None
        };

        if dict.is_some() {
            log::debug!("Dict size for SeqLocs is: {}", dict.as_ref().unwrap().len());
        }

        // todo this should be configurable!
        tree.set_compression(CompressionConfig {
            compression_type: CompressionType::ZSTD,
            compression_level: -3,
            compression_dict: dict,
        });
        tree.write_to_buffer(&mut out_buf)
    }
}

#[derive(
    Debug, Clone, bincode::Encode, bincode::Decode, PartialEq, Eq, Hash,
)]
pub struct Loc
{
    pub block: u32,
    pub start: u32,
    pub len: u32,
}

// TODO: Bytemuck around here somewhere...
impl From<&[u32]> for Loc
{
    fn from(loc: &[u32]) -> Self
    {
        #[cfg(debug_assertions)]
        assert!(loc.len() == 3, "Loc must be 3 u32's long");

        Self {
            block: loc[0],
            start: loc[1],
            len: loc[2],
        }
    }
}

impl Loc
{
    #[inline]
    pub fn get_values(&self) -> [u32; 3]
    {
        [self.block, self.start, self.len]
    }

    #[inline]
    pub fn new(block: u32, start: u32, len: u32) -> Self
    {
        Self { block, start, len }
    }

    #[inline]
    pub fn is_empty(&self) -> bool
    {
        self.len == 0
    }

    /// Slice a loc
    /// Offset from the total sequence should be calculated before
    /// this step But this handles calculating from inside the loc
    /// itself...
    // TODO: Implement for RangeInclusive
    #[inline]
    pub fn slice(&self, range: std::ops::Range<u32>) -> Loc
    {
        Loc {
            block: self.block,
            start: std::cmp::max(
                self.start.saturating_add(range.start),
                self.start,
            ),
            len: range.end.saturating_sub(range.start),
        }
    }
}

pub struct SeqLocsStore
{
    preloaded: bool,
    location: u64,

    #[cfg(not(feature = "async"))]
    tree: FractalTreeDisk<u32, SeqLoc>,

    #[cfg(feature = "async")]
    tree: Arc<FractalTreeDiskAsync<u32, SeqLoc>>,
}

impl SeqLocsStore
{
    #[cfg(not(feature = "async"))]
    /// Prefetch the SeqLocs index into memory. Speeds up successive
    /// access, but can be a hefty one-time cost for large files.
    pub fn prefetch<R>(&mut self, in_buf: &mut R)
    where
        R: Read + Seek + BufRead,
    {
        self.get_all_seqlocs(in_buf)
            .expect("Unable to Prefetch All SeqLocs");

        log::info!("Prefetched {} seqlocs", self.tree.len().unwrap());
    }

    #[cfg(feature = "async")]
    /// Prefetch the SeqLocs index into memory. Speeds up successive
    /// access, but can be a hefty one-time cost for large files.
    #[tracing::instrument(skip(self, in_buf))]
    pub async fn prefetch(
        &self,
        in_buf: &mut tokio::sync::OwnedMutexGuard<
            tokio::io::BufReader<tokio::fs::File>,
        >,
    )
    {
        self.get_all_seqlocs(in_buf)
            .await
            .expect("Unable to Prefetch All SeqLocs");

        log::info!("Prefetched {} seqlocs", self.tree.len().await.unwrap());
    }

    #[cfg(not(feature = "async"))]
    pub fn len<R>(&mut self, in_buf: &mut R) -> usize
    where
        R: Read + Seek + BufRead,
    {
        if !self.preloaded {
            self.get_all_seqlocs(in_buf)
                .expect("Unable to get all SeqLocs");
        }

        self.tree.len().unwrap()
    }

    #[cfg(feature = "async")]
    #[tracing::instrument(skip(self, in_buf))]
    pub async fn len(
        &self,
        in_buf: &mut tokio::sync::OwnedMutexGuard<
            tokio::io::BufReader<tokio::fs::File>,
        >,
    ) -> usize
    {
        if !self.preloaded {
            self.get_all_seqlocs(in_buf)
                .await
                .expect("Unable to get all SeqLocs");
        }

        self.tree.len().await.unwrap()
    }

    #[cfg(not(feature = "async"))]
    /// Get SeqLoc object from a file (buffer)
    pub fn from_existing<R>(pos: u64, in_buf: &mut R) -> Result<Self, String>
    where
        R: Read + Seek + BufRead,
    {
        let store = SeqLocsStore {
            location: pos,
            preloaded: false,
            tree: FractalTreeDisk::from_buffer(in_buf, pos)?,
        };

        Ok(store)
    }

    /// Get SeqLoc object from a file (buffer)
    #[cfg(feature = "async")]
    #[tracing::instrument]
    pub async fn from_existing(
        filename: String,
        pos: u64,
    ) -> Result<Self, String>
    {
        let tree = match FractalTreeDiskAsync::from_buffer(filename, pos).await
        {
            Ok(tree) => Arc::new(tree),
            Err(e) => return Err(e.to_string()),
        };

        let store = SeqLocsStore {
            location: pos,
            preloaded: false,
            tree,
        };

        Ok(store)
    }

    #[cfg(not(feature = "async"))]
    /// Load up all SeqLocs from a file
    pub fn get_all_seqlocs<R>(
        &mut self,
        in_buf: &mut R,
    ) -> Result<(), &'static str>
    where
        R: Read + Seek + BufRead,
    {
        log::info!("Prefetching SeqLocs");

        self.tree.load_tree(in_buf)
    }

    #[cfg(feature = "async")]
    /// Load up all SeqLocs from a file
    #[tracing::instrument(skip(self, in_buf))]
    pub async fn get_all_seqlocs(
        &self,
        in_buf: &mut tokio::sync::OwnedMutexGuard<
            tokio::io::BufReader<tokio::fs::File>,
        >,
    ) -> Result<(), &'static str>
    {
        log::info!("Prefetching SeqLocs");

        Arc::clone(&self.tree).load_tree().await
    }

    #[cfg(not(feature = "async"))]
    /// Get a particular SeqLoc from the store
    pub fn get_seqloc<R>(
        &mut self,
        in_buf: &mut R,
        loc: u32,
    ) -> Result<Option<SeqLoc>, &'static str>
    where
        R: Read + Seek + Send + Sync,
    {
        Ok(self.tree.search(in_buf, &loc).clone())
        // let bincode_config = crate::BINCODE_CONFIG.with_limit::<{
        // 512 * 1024 }>(); // 512kbp

        // in_buf.seek(SeekFrom::Start(self.location + loc as
        // u64)).unwrap(); let seqloc: SeqLoc =
        // bincode::decode_from_std_read(in_buf,
        // bincode_config).unwrap(); Ok(Some(seqloc))
    }

    #[cfg(feature = "async")]
    /// Get a particular SeqLoc from the store
    #[tracing::instrument(skip(self))]
    pub async fn get_seqloc(
        &self,
        loc: u32,
    ) -> Result<Option<SeqLoc>, &'static str>
    {
        Ok(self.tree.search(&loc).await.clone())
    }

    #[cfg(feature = "async")]
    /// Stream all SeqLocs from the store
    pub async fn stream(self: Arc<Self>) -> impl Stream<Item = (u32, SeqLoc)>
    {
        let tree = Arc::clone(&self.tree);
        tree.stream().await
    }
}

#[cfg(test)]
mod tests
{
    use super::*;

    #[test]
    fn test_seqloc_slice()
    {
        let mut seqloc = SeqLoc::new();

        seqloc.sequence = 5;
        seqloc.add_locs(
            &[],
            &vec![
                Loc::new(0, 0, 10),
                Loc::new(1, 0, 10),
                Loc::new(2, 0, 10),
                Loc::new(3, 0, 10),
                Loc::new(4, 0, 10),
            ],
            &[],
            &[],
            &[],
            &[],
            &[],
        );

        println!("{:#?}", seqloc);
        let slice = seqloc.seq_slice(10, 0..10);
        assert_eq!(slice, vec![Loc::new(0, 0, 10)]);
        let slice = seqloc.seq_slice(10, 5..7);
        assert_eq!(slice, vec![Loc::new(0, 5, 2)]);
        let slice = seqloc.seq_slice(10, 15..17);
        assert_eq!(slice, vec![Loc::new(1, 5, 2)]);
        let slice = seqloc.seq_slice(10, 10..20);
        assert_eq!(slice, vec![Loc::new(1, 0, 10)]);
        let slice = seqloc.seq_slice(10, 20..30);
        assert_eq!(slice, vec![Loc::new(2, 0, 10)]);
        let slice = seqloc.seq_slice(10, 15..35);
        assert_eq!(
            slice,
            vec![Loc::new(1, 5, 5), Loc::new(2, 0, 10), Loc::new(3, 0, 5)]
        );
        let slice = seqloc.seq_slice(10, 5..9);
        assert_eq!(slice, vec![Loc::new(0, 5, 4)]);

        let mut seqloc = SeqLoc::new();
        let block_size = 262144;

        seqloc.add_locs(
            &[],
            &vec![
                Loc::new(3097440, 261735, 262144 - 261735),
                Loc::new(3097441, 0, 1274),
            ],
            &[],
            &[],
            &[],
            &[],
            &[],
        );

        //                                  x 261735 ----------> 262144
        // (262144 - 261735) = 409
        //     -------------------------------------------------
        //     <----- 1274
        // = 1274
        //     -------------------------------------------------
        //     So total is 409 + 1274 = 1683
        //
        //     We want 104567 to 104840 -- how?

        let slice = seqloc.seq_slice(block_size, 0..20);
        println!("{:?}", seqloc);
        assert_eq!(slice, vec![Loc::new(3097440, 261735, 20)]);

        let mut seqloc = SeqLoc::new();
        seqloc.add_locs(
            &[],
            &vec![
                Loc::new(1652696, 260695, 262144 - 260695),
                Loc::new(1652697, 0, 28424),
            ],
            &[],
            &[],
            &[],
            &[],
            &[],
        );

        //                               x 260695 ----------> 262144  (262144
        // - 260695) = 1449 -------------------------------------------------
        //   <----- 28424
        // = 28424
        //    -------------------------------------------------
        //    So total is 1449 + 28424 = 29873
        //    We want range 2679 to 2952
        //    So should be the second block
        //    (2, 2679, 2952)

        // 262144 - 260695 = 1449
        // 2679 - 1449 = 1230

        let slice = seqloc.seq_slice(block_size, 2679..2952);
        assert_eq!(slice, vec![Loc::new(1652697, 1230, 2952 - 2679)]);
    }

    // Make sure if seqloc has no header it returns a 0 length &[Loc]
    #[test]
    fn header_zero()
    {
        let seqloc = SeqLoc::new();
        assert_eq!(seqloc.get_headers().len(), 0);
    }

    #[test]
    fn encode_decode_seqloc()
    {
        let mut seqloc = SeqLoc::new();
        seqloc.sequence = 5;
        seqloc.add_locs(
            &vec![
                Loc::new(0, 0, 10),
                Loc::new(1, 0, 10),
                Loc::new(2, 0, 10),
                Loc::new(3, 0, 10),
                Loc::new(4, 0, 10),
            ],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        );

        let mut buf = Vec::new();
        let mut buf = BufWriter::new(&mut buf);

        bincode::encode_into_std_write(
            &seqloc,
            &mut buf,
            crate::BINCODE_CONFIG,
        )
        .unwrap();

        let buf = buf.into_inner().unwrap();
        let mut buf = std::io::Cursor::new(buf);

        let decoded = bincode::decode_from_std_read::<SeqLoc, _, _>(
            &mut buf,
            crate::BINCODE_CONFIG,
        )
        .unwrap();
        assert_eq!(seqloc, decoded);
    }

    #[cfg(not(feature = "async"))]
    #[test]
    pub fn encode_decode_seqlocstore()
    {
        let mut seqloc = SeqLoc::new();
        seqloc.add_locs(
            &vec![
                Loc::new(0, 0, 10),
                Loc::new(1, 0, 10),
                Loc::new(2, 0, 10),
                Loc::new(3, 0, 10),
                Loc::new(4, 0, 10),
            ],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        );

        println!("{:?}", seqloc);

        let mut store = SeqLocsStoreBuilder::new();
        store.add_to_index(seqloc);

        let buf = Vec::new();
        let mut buf = std::io::Cursor::new(buf);
        let mut buf = BufWriter::new(&mut buf);

        store.write_to_buffer(&mut buf).expect("Unable to write");

        let mut buf = buf.into_inner().unwrap();

        let mut store = SeqLocsStore::from_existing(0, &mut buf).unwrap();
        let seqloc = store.get_seqloc(&mut buf, 1).unwrap();
        assert_eq!(seqloc, None);
        let seqloc = store.get_seqloc(&mut buf, 0).unwrap();
        assert_eq!(
            seqloc,
            Some(SeqLoc {
                sequence: 0,
                masking: 0,
                scores: 0,
                signal: 0,
                headers: 0,
                ids: 5,
                mods: 0,
                locs: vec![
                    Loc {
                        block: 0,
                        start: 0,
                        len: 10
                    },
                    Loc {
                        block: 1,
                        start: 0,
                        len: 10
                    },
                    Loc {
                        block: 2,
                        start: 0,
                        len: 10
                    },
                    Loc {
                        block: 3,
                        start: 0,
                        len: 10
                    },
                    Loc {
                        block: 4,
                        start: 0,
                        len: 10
                    }
                ]
            })
        );
    }

    #[test]
    fn test_pos()
    {
        let mut seqloc = SeqLoc::new();
        seqloc.sequence = 5;
        seqloc.add_locs(
            &vec![
                Loc::new(0, 0, 10),
                Loc::new(1, 0, 10),
                Loc::new(2, 0, 10),
                Loc::new(3, 0, 10),
                Loc::new(4, 0, 10),
            ],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        );

        assert_eq!(seqloc.ids_pos(), 0..5);
        assert_eq!(seqloc.sequence_pos(), 5..5);
        assert_eq!(seqloc.masking_pos(), 5..5);
        assert_eq!(seqloc.scores_pos(), 5..5);
        assert_eq!(seqloc.signal_pos(), 5..5);
        assert_eq!(seqloc.headers_pos(), 5..5);
        assert_eq!(seqloc.mods_pos(), 5..5);

        let mut seqloc = SeqLoc::new();
        let dummy_locs = vec![
            Loc::new(0, 0, 10),
            Loc::new(1, 0, 10),
            Loc::new(2, 0, 10),
            Loc::new(3, 0, 10),
            Loc::new(4, 0, 10),
        ];
        seqloc.add_locs(
            &dummy_locs,
            &dummy_locs,
            &dummy_locs,
            &dummy_locs,
            &dummy_locs,
            &dummy_locs,
            &dummy_locs,
        );

        assert_eq!(seqloc.ids_pos(), 0..5);
        assert_eq!(seqloc.sequence_pos(), 5..10);
        assert_eq!(seqloc.masking_pos(), 10..15);
        assert_eq!(seqloc.scores_pos(), 15..20);
        assert_eq!(seqloc.signal_pos(), 20..25);
        assert_eq!(seqloc.headers_pos(), 25..30);
        assert_eq!(seqloc.mods_pos(), 30..35);
    }
}
