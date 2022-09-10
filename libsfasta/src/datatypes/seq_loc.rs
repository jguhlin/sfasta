// TODO! Need tests...
// TODO: Bitpack seqlocs instead of zstd compress...
// TODO: When data gets too large, pre-emptively compress it into memory (such as nt db, >200Gb).

use std::io::{Read, Seek, SeekFrom, Write};

/// Handles access to SeqLocs
pub struct SeqLocs {
    location: u64,
    block_index_pos: u64,
    block_locations: Option<Vec<u64>>,
    chunk_size: usize,
    pub data: Option<Vec<SeqLoc>>, // Primarily for writing...
    len: usize,
    cache: Option<(u32, Vec<SeqLoc>)>,
}

impl Default for SeqLocs {
    fn default() -> Self {
        SeqLocs {
            location: 0,
            block_index_pos: 0,
            block_locations: None,
            chunk_size: 256 * 1024,
            data: None,
            len: 0,
            cache: None,
        }
    }
}

impl SeqLocs {
    pub fn new() -> Self {
        SeqLocs::default()
    }

    pub fn prefetch<R>(&mut self, mut in_buf: &mut R)
    where
        R: Read + Seek,
    {
        let mut data = Vec::with_capacity(self.len());
        log::debug!(
            "Total SeqLoc Blocks: {}",
            self.block_locations.as_ref().unwrap().len()
        );

        for i in 0..self.block_locations.as_ref().unwrap().len() {
            log::debug!("Reading SeqLoc Block: {}", i);
            let seq_loc = self.get_block_uncached(&mut in_buf, i as u32);
            data.extend(seq_loc);
        }

        self.data = Some(data);

        log::debug!("Prefetched {} seqlocs", self.len());
    }

    pub fn with_data(data: Vec<SeqLoc>) -> Self {
        SeqLocs {
            location: 0,
            block_index_pos: 0,
            block_locations: None,
            chunk_size: 256 * 1024,
            data: Some(data),
            len: 0,
            cache: None,
        }
    }

    pub fn with_location(mut self, location: u64) -> Self {
        self.location = location;
        self
    }

    pub fn with_block_index_pos(mut self, block_index_pos: u64) -> Self {
        self.block_index_pos = block_index_pos;
        self
    }

    pub fn with_chunk_size(mut self, chunk_size: usize) -> Self {
        self.chunk_size = chunk_size;
        self
    }

    pub fn write_to_buffer<W>(&mut self, mut out_buf: &mut W) -> u64
    where
        W: Write + Seek,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        if self.data.is_none() {
            panic!("Unable to write SeqLocs as there are none");
        }

        let starting_pos = out_buf.seek(SeekFrom::Current(0)).unwrap();

        let seq_locs = self.data.take().unwrap();

        let total_seq_locs = seq_locs.len() as u64;

        let seqlocs_location = out_buf
            .seek(SeekFrom::Current(0))
            .expect("Unable to work with seek API");

        let mut block_locations: Vec<u64> =
            Vec::with_capacity((seq_locs.len() / self.chunk_size) + 1);

        // Write out chunk sizes...
        bincode::encode_into_std_write(&self.chunk_size, &mut out_buf, bincode_config)
            .expect("Unable to write out chunk size");

        // Write out block index pos
        bincode::encode_into_std_write(&self.block_index_pos, &mut out_buf, bincode_config)
            .expect("Unable to write out chunk size");

        // Write out the number of sequences...
        bincode::encode_into_std_write(&total_seq_locs, &mut out_buf, bincode_config)
            .expect("Unable to write out chunk size");

        // zstd level -3 for speed
        // zstd appears to outperform lz4 for numeric data
        //let mut compressor = zstd_encoder(9);
        //let mut compressor = zstd::stream::Encoder::new(Vec::with_capacity(2 * 1024 * 1024), -3).unwrap();
        // zstd::bulk::Compressor::new(9).unwrap();

        // FORMAT: Write sequence location blocks
        // TODO: Make a chunk for this, and split up strings + numbers, and bincode the numbers...
        for s in seq_locs
            .iter()
            .collect::<Vec<&SeqLoc>>()
            .chunks(self.chunk_size)
        {
            block_locations.push(
                out_buf
                    .seek(SeekFrom::Current(0))
                    .expect("Unable to work with seek API"),
            );

            let locs = s.to_vec();

            let mut bincoded: Vec<u8> = Vec::new();

            let mut compressor = zstd::stream::Encoder::new(Vec::with_capacity(2 * 1024 * 1024), 3)
                .expect("Unable to create zstd encoder");
            compressor.include_magicbytes(false).unwrap();
            compressor.long_distance_matching(true).unwrap();

            bincode::encode_into_std_write(&locs, &mut bincoded, bincode_config)
                .expect("Unable to bincode locs into compressor");

            compressor.write_all(&bincoded).unwrap();
            let compressed = compressor.finish().unwrap();

            bincode::encode_into_std_write(compressed, &mut out_buf, bincode_config)
                .expect("Unable to write Sequence Blocks to file");
        }

        self.block_index_pos = out_buf
            .seek(SeekFrom::Current(0))
            .expect("Unable to work with seek API");

        // Does this need a dual index or bitpacking?
        // Need to measure on large files...
        // let mut compressor =
        // zstd::stream::Encoder::new(Vec::with_capacity(8 * 1024 * 1024), -3).unwrap();

        let mut bincoded: Vec<u8> = Vec::new();

        bincode::encode_into_std_write(&block_locations, &mut bincoded, bincode_config)
            .expect("Unable to bincode locs into compressor");

        let mut compressor = zstd::stream::Encoder::new(Vec::with_capacity(2 * 1024 * 1024), -3)
            .expect("Unable to create zstd encoder");
        compressor.include_magicbytes(false).unwrap();
        compressor.long_distance_matching(true).unwrap();

        compressor.write_all(&bincoded).unwrap();
        let compressed = compressor.finish().unwrap();

        bincode::encode_into_std_write(compressed, &mut out_buf, bincode_config)
            .expect("Unable to write Sequence Blocks to file");

        self.block_locations = Some(block_locations);

        let end = out_buf
            .seek(SeekFrom::Current(0))
            .expect("Unable to work with seek API");

        out_buf.seek(SeekFrom::Start(starting_pos)).unwrap();

        // Write out chunk sizes...
        bincode::encode_into_std_write(&self.chunk_size, &mut out_buf, bincode_config)
            .expect("Unable to write out chunk size");

        // Write out block index pos
        bincode::encode_into_std_write(&self.block_index_pos, &mut out_buf, bincode_config)
            .expect("Unable to write out chunk size");

        bincode::encode_into_std_write(&total_seq_locs, &mut out_buf, bincode_config)
            .expect("Unable to write out chunk size");

        out_buf.seek(SeekFrom::Start(end)).unwrap();

        seqlocs_location
    }

    pub fn from_buffer<R>(mut in_buf: &mut R, pos: u64) -> Self
    where
        R: Read + Seek,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        in_buf
            .seek(SeekFrom::Start(pos))
            .expect("Unable to work with seek API");

        let chunk_size: usize = bincode::decode_from_std_read(&mut in_buf, bincode_config)
            .expect("Unable to read chunk size");

        let block_index_pos = bincode::decode_from_std_read(&mut in_buf, bincode_config)
            .expect("Unable to read block index pos");

        let len: u64 = bincode::decode_from_std_read(&mut in_buf, bincode_config)
            .expect("Unable to read block index pos");

        in_buf.seek(SeekFrom::Start(block_index_pos)).unwrap();
        let compressed_block_locations: Vec<u8> =
            bincode::decode_from_std_read(&mut in_buf, bincode_config)
                .expect("Unable to read block locations");

        let mut decompressor =
            zstd::stream::read::Decoder::new(&compressed_block_locations[..]).unwrap();
        decompressor.include_magicbytes(false).unwrap();

        let mut decompressed = Vec::new();
        decompressor.read_to_end(&mut decompressed).unwrap();

        let block_locations: Vec<u64> =
            bincode::decode_from_std_read(&mut decompressed.as_slice(), bincode_config)
                .expect("Unable to read block locations");
        SeqLocs {
            location: pos,
            block_index_pos,
            block_locations: Some(block_locations),
            chunk_size,
            data: None, // Don't decompress anything until requested...
            len: len as usize,
            cache: None,
        }
    }

    pub fn is_initialized(&self) -> bool {
        self.block_locations.is_some()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn get_block<R>(&mut self, in_buf: &mut R, block: u32) -> &[SeqLoc]
    where
        R: Read + Seek,
    {
        if !self.is_initialized() {
            panic!("Unable to get SeqLoc as SeqLocs are not initialized");
        }

        if self.cache.is_some() && self.cache.as_ref().unwrap().0 == block {
            let cache = self.cache.as_ref().unwrap();
            &cache.1
        } else {
            let seqlocs = self.get_block_uncached(in_buf, block);

            self.cache = Some((block as u32, seqlocs));

            &self.cache.as_ref().unwrap().1
        }
    }

    fn get_block_uncached<R>(&self, mut in_buf: &mut R, block: u32) -> Vec<SeqLoc>
    where
        R: Read + Seek,
    {
        let block_locations = self.block_locations.as_ref().unwrap();
        let block_location = block_locations[block as usize];
        in_buf.seek(SeekFrom::Start(block_location)).unwrap();

        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        let compressed_block: Vec<u8> = bincode::decode_from_std_read(&mut in_buf, bincode_config)
            .expect("Unable to read block");
        let mut decompressor = zstd::stream::read::Decoder::new(&compressed_block[..]).unwrap();
        decompressor.include_magicbytes(false).unwrap();

        let mut decompressed = Vec::with_capacity(8 * 1024 * 1024);

        decompressor.read_to_end(&mut decompressed).unwrap();

        let seqlocs: Vec<SeqLoc> =
            bincode::decode_from_std_read(&mut decompressed.as_slice(), bincode_config)
                .expect("Unable to read block");
        seqlocs
    }

    pub fn get_seqloc<R>(
        &mut self,
        in_buf: &mut R,
        index: u32,
    ) -> Result<Option<SeqLoc>, &'static str>
    where
        R: Read + Seek,
    {
        if !self.is_initialized() {
            return Err("Unable to get SeqLoc as SeqLocs are not initialized");
        }

        if self.data.is_some() {
            Ok(Some(self.data.as_ref().unwrap()[index as usize].clone()))
        } else {
            let block = index / self.chunk_size as u32;
            let block_index = index % self.chunk_size as u32;
            let seqlocs = self.get_block(in_buf, block);
            Ok(Some(seqlocs[block_index as usize].clone()))
        }
    }
}

#[derive(Debug, Clone, bincode::Encode, bincode::Decode, Default, PartialEq, Eq, Hash)]
pub struct SeqLoc {
    pub sequence: Option<Vec<Loc>>,
    pub masking: Option<(u32, u32)>,
    pub scores: Option<Vec<Loc>>,
    pub headers: Option<Vec<Loc>>,
    pub ids: Option<Vec<Loc>>,
}

impl SeqLoc {
    pub const fn new() -> Self {
        Self {
            sequence: None,
            masking: None,
            scores: None,
            headers: None,
            ids: None,
        }
    }

    pub fn len(&self, block_size: u32) -> usize {
        if self.sequence.is_some() {
            self.sequence.as_ref().unwrap().iter().map(|x| x.len(block_size)).sum::<usize>()
        } else if self.masking.is_some() {
            self.masking.as_ref().unwrap().1 as usize
        } else if self.scores.is_some() {
            self.scores.as_ref().unwrap().len()
        } else {
            0
        }
    }

    // Convert range to locs to slice
    // Have to map ranges to the Vec<Locs>
    pub fn slice(&self, block_size: u32, range: std::ops::Range<usize>) -> SeqLoc {
        assert!(self.sequence.is_some());
        let locs = self.sequence.as_ref().unwrap();
        let mut new_locs = Vec::new();
        let slice_length = range.end - range.start;
        let start_block_ordinal = range.start / block_size as usize;
        let end_block_ordinal = (range.end - 1) / block_size as usize;

        let start_block_offset = range.start % block_size as usize;
        let end_block_offset = (range.end - 1) % block_size as usize;

        if start_block_ordinal == end_block_ordinal {
            let loc = &locs[start_block_ordinal as usize];
            let (block, (start, _end)) = loc.original_format(block_size);
            println!("{} {} {}", block, start, _end);
            println!("{} {} {} {}", start_block_ordinal, start_block_offset, end_block_ordinal, end_block_offset);

            new_locs.push(Loc::Loc(block, start + start_block_offset as u32,
                start + end_block_offset as u32 + 1));

        } else {
            let start_loc = &locs[start_block_ordinal as usize];
            let (block, (start, end)) = start_loc.original_format(block_size);
            new_locs.push(Loc::Loc(block, start + start_block_offset as u32, end));

            for i in start_block_ordinal + 1..end_block_ordinal {
                new_locs.push(locs[i as usize].clone());
            }

            let end_loc = &locs[end_block_ordinal as usize];
            let (block, (start, _end)) = end_loc.original_format(block_size);
            new_locs.push(Loc::Loc(block, start, start + end_block_offset as u32 + 1));
        }

        assert!(new_locs.iter().map(|x| x.len(block_size)).sum::<usize>() == slice_length);

        SeqLoc {
            sequence: Some(new_locs),
            masking: None,
            scores: None,
            headers: self.headers.clone(),
            ids: self.ids.clone(),
        }

    }
}

#[derive(Debug, Clone, bincode::Encode, bincode::Decode, PartialEq, Eq, Hash)]
pub enum Loc {
    Loc(u32, u32, u32),  // Block, start, end...
    FromStart(u32, u32), // Block, length...
    ToEnd(u32, u32),     // Block, start,
    EntireBlock(u32),    // Entire block belongs to this seq...
                         // pub block: u32,
                         // pub start: u32,
                         // pub end: u32,
}

impl Loc {
    /// The ultimate in lazy programming. This was once a type (tuple) and is now a enum....
    #[inline]
    pub fn original_format(&self, block_size: u32) -> (u32, (u32, u32)) {
        match self {
            Loc::Loc(block, start, end) => (*block, (*start, *end)),
            Loc::FromStart(block, length) => (*block, (0, *length)),
            Loc::ToEnd(block, start) => (*block, (*start, block_size)),
            Loc::EntireBlock(block) => (*block, (0, block_size)),
        }
    }

    pub fn len(&self, block_size: u32) -> usize {
        match self {
            Loc::Loc(_, start, end) => (*end - *start) as usize,
            Loc::FromStart(_, length) => *length as usize,
            Loc::ToEnd(_, start) => block_size as usize - *start as usize,
            Loc::EntireBlock(_) => block_size as usize,
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Loc::Loc(_, start, end) => *start == *end,
            Loc::FromStart(_, length) => *length == 0,
            Loc::ToEnd(_, start) => *start == 0,
            Loc::EntireBlock(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_original_format() {
        let loc = Loc::Loc(0, 1, 2);
        assert_eq!(loc.original_format(10), (0, (1, 2)));
        let loc = Loc::FromStart(0, 2);
        assert_eq!(loc.original_format(10), (0, (0, 2)));
        let loc = Loc::ToEnd(0, 2);
        assert_eq!(loc.original_format(10), (0, (2, 10)));
        let loc = Loc::EntireBlock(0);
        assert_eq!(loc.original_format(10), (0, (0, 10)));
    }

    #[test]
    fn test_seqloc_slice() {
        let mut seqloc = SeqLoc::new();
        seqloc.sequence = Some(vec![
            Loc::Loc(0, 0, 10),
            Loc::Loc(1, 0, 10),
            Loc::Loc(2, 0, 10),
            Loc::Loc(3, 0, 10),
            Loc::Loc(4, 0, 10)]);
        let slice = seqloc.slice(10, 0..10);
        assert_eq!(slice.sequence, Some(vec![Loc::Loc(0, 0, 10)]));
        let slice = seqloc.slice(10, 10..20);
        assert_eq!(slice.sequence, Some(vec![Loc::Loc(1, 0, 10)]));
        let slice = seqloc.slice(10, 20..30);
        assert_eq!(slice.sequence, Some(vec![Loc::Loc(2, 0, 10)]));
        let slice = seqloc.slice(10, 15..35);
        assert_eq!(slice.sequence, Some(vec![Loc::Loc(1, 5, 10), Loc::Loc(2, 0, 10), Loc::Loc(3, 0, 5)]));
        let slice = seqloc.slice(10, 5..9);
        assert_eq!(slice.sequence, Some(vec![Loc::Loc(0, 5, 9)]));

        let block_size = 262144;
        seqloc.sequence = Some(vec![
            Loc::ToEnd(3097440, 261735),
            Loc::FromStart(3097441, 1274),
        ]);

        println!("Length: {}", seqloc.len(block_size));

        //                                  x 261735 ----------> 262144  (262144 - 261735) = 409
        //     -------------------------------------------------
        //     <----- 1274                                                                 = 1274
        //     -------------------------------------------------
        //     So total is 409 + 1274 = 1683
        //
        //     We want 104567 to 104840 -- how?



        let slice = seqloc.slice(block_size, 104567..104840);
        assert_eq!(slice.sequence, Some(vec![Loc::Loc(3097440, 261735, 262144), Loc::Loc(3097441, 0, 1274)]));



    }
}