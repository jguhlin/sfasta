//! SeqLocs handle the location pointers for sequences, masks, IDs, etc...
//! Each SeqLoc will point to a start and a length of Loc objects for a given sequence
//!
//! A Loc is a pointer to a sequence, and the start of it and length, which are all u32's and found in the struct Loc
//!
//! SeqLocsStore is the collection of all seqlocs
//! SeqLocs are the complete data for a specific sequence
//! Locs are the pointers to the sequence data (and strings, etc)
//!
//! SeqLocs are pointed to from the index (FractalTree<u64, (&str, u64))
//! todo: make u32 for the start of the seq in the file?

// Stored as a row, as we will often need to decode everything

// Sequences (ids, masking, scores, header, etc...)
// are stored in blocks, and then the range of the sequence
// SeqLoc stores Locs, which are (u32, u32, u32) (block, start, len) as struct Loc

use std::{
    io::{Read, Seek, SeekFrom, Write},
    sync::{atomic::AtomicU64, Arc},
};

use stream_vbyte::{decode::decode, encode::encode, scalar::Scalar};

// So each SeqLoc is:
// Each seq, masking, scores, header, ids, are the number for each type
// Then the Locs are stored in a Vec<Loc>
// So masking Locs are found as locs[sequence..sequence+masking]
// Compressed version is simply Vec<u8> of all the Locs
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SeqLoc
{
    pub sequence: u16,
    pub masking: u16,
    pub scores: u16,
    pub signal: u16, // FUTURE: Add Nanopore signals (or others)
    pub headers: u16,
    pub ids: u16,
    pub locs: Vec<Loc>,
}

// Optimized on-disk storage
#[derive(Clone, Debug, bincode::Encode, bincode::Decode, Default, PartialEq, Eq)]
pub struct SeqLocOnDisk
{
    pub values: Vec<u8>,
    pub compressed: Vec<u8>,
}
// SeqLocOnDisk is a Vec<u32> of all the Loc data
// Each u16 is the number of Locs / 3 (block, start, len) for each SeqLoc type
// Integers can then be compressed with Stream VByte or other integer compression technique

impl From<SeqLoc> for SeqLocOnDisk
{
    fn from(seqloc: SeqLoc) -> Self
    {
        let SeqLoc {
            sequence,
            masking,
            scores,
            signal,
            headers,
            ids,
            locs,
        } = seqloc;

        let total_length = (sequence + masking + scores + signal + headers + ids) * 3;

        let mut compressed: Vec<u8> = vec![0; total_length as usize];

        // TODO: Is there a way to directly access the Vec<Loc> to get Vec<u32>'s?
        let nums = locs.iter().map(|loc| loc.get_values()).flatten().collect::<Vec<u32>>();

        let encoded_length = encode::<Scalar>(&nums, &mut compressed);
        compressed.truncate(encoded_length);

        // Todo: Does this actually save any space?
        let values_uncompressed = vec![sequence, masking, scores, signal, headers, ids];

        let values_uncompressed = unsafe { std::mem::transmute::<Vec<u16>, Vec<u32>>(values_uncompressed) };

        let mut values = vec![0; values_uncompressed.len() * 5];
        let encoded_length = encode::<Scalar>(&values_uncompressed, &mut values);
        values.truncate(encoded_length);

        SeqLocOnDisk { values, compressed }
    }
}

// Todo: compress seq, masking, scores, signal, headers, ids
impl From<SeqLocOnDisk> for SeqLoc
{
    fn from(ondisk: SeqLocOnDisk) -> Self
    {
        let SeqLocOnDisk { values, compressed } = ondisk;

        let mut values_uncompressed: Vec<u32> = vec![0; 6];
        decode::<Scalar>(&values, 6, &mut values_uncompressed);
        let values = unsafe { std::mem::transmute::<Vec<u32>, Vec<u16>>(values_uncompressed) };

        let sequence = values[0];
        let masking = values[1];
        let scores = values[2];
        let signal = values[3];
        let headers = values[4];
        let ids = values[5];

        let integers_len = (sequence + masking + scores + signal + headers + ids) as usize * 3;

        let mut decoded = vec![0; integers_len];

        // Todo: encode/decode benefit from pulp?
        // See if any benefit...
        decode::<Scalar>(&compressed, integers_len, &mut decoded);

        let locs = decoded
            .chunks_exact(3)
            .map(|chunk| Loc::from(chunk))
            .collect::<Vec<Loc>>();

        SeqLoc {
            sequence,
            masking,
            scores,
            signal,
            headers,
            ids,
            locs,
        }
    }
}

// pub sequence: Option<(u64, u32)>,
// pub masking: Option<(u64, u32)>,
// pub scores: Option<(u64, u32)>,
// pub headers: Option<(u64, u8)>,
// pub ids: Option<(u64, u8)>,

// TODO! Need tests...
// TODO: When data gets too large, pre-emptively compress it into memory (such as nt db, >200Gb).
// TODO: Flatten seqlocs into a single vec, then use ordinals to find appropritate ones
// TODO: Can convert this to use ByteBlockStore?

/// Handles access to SeqLocs
#[derive(Clone)]
pub struct SeqLocsStoreBuilder
{
    pub location: u64,
    pub data: Vec<(SeqLoc, Arc<AtomicU64>)>,
}

impl Default for SeqLocsStoreBuilder
{
    fn default() -> Self
    {
        SeqLocsStoreBuilder {
            location: 0,
            data: Vec::new(),
        }
    }
}

impl SeqLocsStoreBuilder
{
    /// Create a new SeqLocs object
    pub fn new() -> Self
    {
        SeqLocsStoreBuilder::default()
    }

    /// Add a SeqLoc to the store
    pub fn add_to_index(&mut self, seqloc: SeqLoc) -> Arc<AtomicU64>
    {
        let location = Arc::new(AtomicU64::new(0));
        self.data.push((seqloc, Arc::clone(&location)));
        location
    }

    /// Set the location u64 of the SeqLocs object
    pub fn with_location(mut self, location: u64) -> Self
    {
        self.location = location;
        self
    }

    /// Write a SeqLocs object to a file (buffer)
    pub fn write_to_buffer<W>(&mut self, mut out_buf: &mut W) -> u64
    where
        W: Write + Seek,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        if self.data.is_empty() {
            panic!("Unable to write SeqLocs as there are none");
        }

        self.location = out_buf.stream_position().unwrap();

        let mut data = Vec::new();
        std::mem::swap(&mut self.data, &mut data);

        for (seqloc, location) in data.into_iter() {
            let seqloc = SeqLocOnDisk::from(seqloc);
            let pos = out_buf.stream_position().expect("Unable to work with seek API");
            bincode::encode_into_std_write(seqloc, &mut out_buf, bincode_config)
                .expect("Unable to write SeqLoc to file");
            location.store(pos, std::sync::atomic::Ordering::Relaxed);
        }

        self.location
    }
}

impl SeqLoc
{
    pub const fn new() -> Self
    {
        Self {
            sequence: 0,
            masking: 0,
            scores: 0,
            signal: 0,
            headers: 0,
            ids: 0,
            locs: Vec::new(),
        }
    }

    #[inline(always)]
    fn masking_pos(&self) -> usize
    {
        self.sequence as usize
    }

    #[inline(always)]
    fn scores_pos(&self) -> usize
    {
        (self.sequence + self.masking) as usize
    }

    #[inline(always)]
    fn signal_pos(&self) -> usize
    {
        (self.sequence + self.masking + self.scores) as usize
    }

    #[inline(always)]
    fn headers_pos(&self) -> usize
    {
        (self.sequence + self.masking + self.scores + self.signal) as usize
    }

    #[inline(always)]
    fn ids_pos(&self) -> usize
    {
        (self.sequence + self.masking + self.scores + self.signal + self.headers) as usize
    }

    pub fn get_sequence(&self) -> &[Loc]
    {
        &self.locs[0..self.sequence as usize]
    }

    pub fn get_masking(&self) -> &[Loc]
    {
        &self.locs[self.sequence as usize..self.masking_pos()]
    }

    pub fn get_scores(&self) -> &[Loc]
    {
        &self.locs[self.masking_pos()..self.scores_pos()]
    }

    pub fn get_signal(&self) -> &[Loc]
    {
        &self.locs[self.scores_pos()..self.signal_pos()]
    }

    pub fn get_headers(&self) -> &[Loc]
    {
        &self.locs[self.signal_pos()..self.headers_pos()]
    }

    pub fn get_ids(&self) -> &[Loc]
    {
        &self.locs[self.headers_pos()..self.ids_pos()]
    }

    pub fn add_masking_locs(&mut self, locs: Vec<Loc>)
    {
        self.masking = locs.len() as u16;
        self.locs.extend(locs);
    }

    pub fn add_sequence_locs(&mut self, locs: Vec<Loc>)
    {
        self.sequence = locs.len() as u16;
        self.locs.extend(locs);
    }

    pub fn add_header_locs(&mut self, locs: Vec<Loc>)
    {
        self.headers = locs.len() as u16;
        self.locs.extend(locs);
    }

    pub fn add_id_locs(&mut self, locs: Vec<Loc>)
    {
        self.ids = locs.len() as u16;
        self.locs.extend(locs);
    }

    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize
    {
        self.locs.iter().map(|loc| loc.len as usize).sum()
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
    // TODO: Should work on staggered Locs, even though they do not exist....
    pub fn seq_slice(&self, block_size: u32, range: std::ops::Range<u32>) -> Vec<Loc>
    {
        let mut new_locs = Vec::new();

        let locs = &self.locs;

        let splits = SeqLoc::seq_location_splits(&locs);

        let end = range.end - 1;

        for (i, split) in splits.iter().enumerate() {
            if split.contains(&range.start) && split.contains(&end) {
                // This loc contains the entire range
                // So for example, Loc is 1500..2000, and the range we want is 20..50 (translates to 1520..1550)
                let start = range.start.saturating_sub(split.start);
                let end = range.end.saturating_sub(split.start);
                new_locs.push(locs[i].slice(start..end));
                break; // We are done if it contains the entire range...
            } else if split.contains(&range.start) {
                // Loc contains the start of the range...
                // For example, Loc is 1500..2000 (length 500 in this Loc) and the range we want is 450..550 (so
                // 1950..2000 from this loc, and another 100 from the next loc)
                let start = range.start.saturating_sub(split.start);
                let end = block_size; // range.end.saturating_sub(split.start);
                new_locs.push(locs[i].slice(start..end));
            } else if split.contains(&end) {
                // Loc contains the end of the range...
                // For example, Loc is 1500..2000 (length 500 in this Loc, starting at 1000) and the range we want is
                // 900..1200 (so 1500..1700 from this loc)
                let start = range.start.saturating_sub(split.start);
                let end = range.end.saturating_sub(split.start);
                new_locs.push(locs[i].slice(start..end));
                break; // We are done if it contains the end of the range...
            } else if split.start > range.start && split.end < range.end {
                // Loc contains the entire range...
                // For example, Loc is 1500..2000 (length 500 in the Loc) and the range we want is 450..550 (so
                // 1500..1550 from this loc, and another 100 from the previous loc)
                new_locs.push(locs[i].clone());
            } else {
                // Loc does not contain the range...
                // For example, Loc is 1500..2000 (length 500 in the Loc) and the range we want is 250..350 (so
                // 1750..1800 from this loc)
            }
        }

        new_locs
    }
}

#[derive(Debug, Clone, bincode::Encode, bincode::Decode, PartialEq, Eq, Hash)]
pub struct Loc
{
    pub block: u32,
    pub start: u32,
    pub len: u32,
}

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
    /// Offset from the total sequence should be calculated before this step
    /// But this handles calculating from inside the loc itself...
    // TODO: Implement for RangeInclusive
    #[inline]
    pub fn slice(&self, range: std::ops::Range<u32>) -> Loc
    {
        Loc {
            block: self.block,
            start: std::cmp::max(self.start.saturating_add(range.start), self.start),
            len: range.end.saturating_sub(range.start),
        }
    }
}

#[derive(Clone)]
pub struct SeqLocsStore
{
    location: u64,
    data: Vec<SeqLoc>,
    locations: Vec<u64>, // On-disk location of each seqloc, useful for finding the right one
    // when they are all loaded into memory...
    // Use binary search
    preloaded: bool,
}

impl SeqLocsStore
{
    /// Prefetch the SeqLocs index into memory. Speeds up successive access, but can be a hefty one-time cost for large
    /// files.
    pub fn prefetch<R>(&mut self, mut in_buf: &mut R)
    where
        R: Read + Seek,
    {
        self.get_all_seqlocs(&mut in_buf)
            .expect("Unable to Prefetch All SeqLocs");

        self.preloaded = true;

        log::info!("Prefetched {} seqlocs", self.data.len());
    }

    /// Get SeqLoc object from a file (buffer)
    pub fn from_existing(pos: u64) -> Result<Self, String>
    {
        let store = SeqLocsStore {
            location: pos,
            data: Vec::new(),
            locations: Vec::new(),
            preloaded: false,
        };

        Ok(store)
    }

    /// Load up all SeqLocs from a file
    pub fn get_all_seqlocs<R>(&mut self, mut in_buf: &mut R) -> Result<&Vec<SeqLoc>, &'static str>
    where
        R: Read + Seek,
    {
        log::info!("Prefetching SeqLocs");

        let bincode_config = bincode::config::standard()
            .with_fixed_int_encoding()
            .with_limit::<{ 8 * 1024 * 1024 }>(); // 8Mbp

        in_buf.seek(SeekFrom::Start(self.location)).unwrap();

        // Basically keep going until we get an error, and assume that's the EOF
        // or a different data type...
        while let Ok(seqloc) = bincode::decode_from_std_read::<SeqLocOnDisk, _, _>(&mut in_buf, bincode_config) {
            let seqloc = SeqLoc::from(seqloc);
            let pos = in_buf.stream_position().unwrap();
            self.locations.push(pos);
            self.data.push(seqloc);
        }

        log::debug!("Finished");
        return Ok(&self.data);
    }

    /// Get a particular SeqLoc from the store
    pub fn get_seqloc<R>(&mut self, in_buf: &mut R, loc: u32) -> Result<Option<SeqLoc>, &'static str>
    where
        R: Read + Seek,
    {
        let bincode_config = bincode::config::standard()
            .with_fixed_int_encoding()
            .with_limit::<{ 8 * 1024 * 1024 }>(); // 8Mbp

        in_buf.seek(SeekFrom::Start(self.location + loc as u64)).unwrap();
        let seqloc: Result<SeqLocOnDisk, _> = bincode::decode_from_std_read(in_buf, bincode_config);
        match seqloc {
            Ok(seqloc) => Ok(Some(SeqLoc::from(seqloc))),
            Err(_) => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests
{
    use super::*;

    #[test]
    fn test_seqloc_slice()
    {
        let mut seqlocs = SeqLocsStoreBuilder::default();
        let mut seqloc = SeqLoc::new();

        let mut dummy_buffer = std::io::Cursor::new(vec![0; 1024]);

        seqloc.sequence = 5;
        seqloc.locs.extend(vec![
            Loc::new(0, 0, 10),
            Loc::new(1, 0, 10),
            Loc::new(2, 0, 10),
            Loc::new(3, 0, 10),
            Loc::new(4, 0, 10),
        ]);

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
        assert_eq!(slice, vec![Loc::new(1, 5, 5), Loc::new(2, 0, 10), Loc::new(3, 0, 5)]);
        let slice = seqloc.seq_slice(10, 5..9);
        assert_eq!(slice, vec![Loc::new(0, 5, 4)]);
        let block_size = 262144;
        seqloc.sequence += 2;

        seqloc.locs.extend(vec![
            Loc::new(3097440, 261735, 262144 - 261735),
            Loc::new(3097441, 0, 1274),
        ]);

        //                                  x 261735 ----------> 262144  (262144 - 261735) = 409
        //     -------------------------------------------------
        //     <----- 1274                                                                 = 1274
        //     -------------------------------------------------
        //     So total is 409 + 1274 = 1683
        //
        //     We want 104567 to 104840 -- how?

        let slice = seqloc.seq_slice(block_size, 0..20);
        assert_eq!(slice, vec![Loc::new(3097440, 261735, 20)]);

        seqloc.sequence += 2;

        seqloc.locs.extend(vec![
            Loc::new(1652696, 260695, 262144 - 260695),
            Loc::new(1652697, 0, 28424),
        ]);

        //                               x 260695 ----------> 262144  (262144 - 260695) = 1449
        //    -------------------------------------------------
        //    <----- 28424                                                              = 28424
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
}
