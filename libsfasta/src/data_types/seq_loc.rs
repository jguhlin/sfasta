// TODO! Need tests...
// TODO: Bitpack seqlocs instead of zstd compress...

use std::io::{Read, Seek, SeekFrom, Write};

use crate::data_types::sequence_block::zstd_encoder;

/// Handles access to SeqLocs
pub struct SeqLocs {
    location: u64,
    block_index_pos: u64,
    block_locations: Option<Vec<u64>>,
    chunk_size: usize,
    pub data: Option<Vec<SeqLoc>>, // Only used for writing...
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

            let mut compressor = zstd::stream::Encoder::new(Vec::with_capacity(2 * 1024 * 1024), 9)
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

    pub fn get_seqloc<R>(&mut self, mut in_buf: &mut R, index: u32) -> SeqLoc
    where
        R: Read + Seek,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        if !self.is_initialized() {
            panic!("Unable to get SeqLoc as SeqLocs are not initialized");
        }

        let index: usize = index as usize;

        let block_locations = self.block_locations.as_ref().unwrap();

        let block_index = index / self.chunk_size;
        let block_offset = index % self.chunk_size;

        if self.cache.is_some() {
            let cache = self.cache.as_ref().unwrap();
            if cache.0 == block_index as u32 {
                return cache.1[block_offset].clone();
            }
        }

        let block_location = block_locations[block_index];

        in_buf.seek(SeekFrom::Start(block_location)).unwrap();

        let compressed_block: Vec<u8> = bincode::decode_from_std_read(&mut in_buf, bincode_config)
            .expect("Unable to read block");

        let mut decompressor = zstd::stream::read::Decoder::new(&compressed_block[..]).unwrap();
        decompressor.include_magicbytes(false).unwrap();

        let mut decompressed = Vec::with_capacity(8 * 1024 * 1024);

        decompressor.read_to_end(&mut decompressed).unwrap();

        let seqlocs: Vec<SeqLoc> =
            bincode::decode_from_std_read(&mut decompressed.as_slice(), bincode_config)
                .expect("Unable to read block");

        self.cache = Some((block_index as u32, seqlocs));

        self.cache.as_ref().unwrap().1[block_offset].clone()
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
}

#[derive(Debug, Clone, bincode::Encode, bincode::Decode, Default, PartialEq, Eq, Hash)]
pub struct Loc {
    pub block: u32,
    pub start: u32,
    pub end: u32,
}

impl Loc {
    /// The ultimate in lazy programming. This was once a type (tuple) and is now a struct...
    pub fn original_format(&self) -> (u32, (u32, u32)) {
        (self.block, (self.start, self.end))
    }

    pub fn new(block: u32, start: u32, end: u32) -> Self {
        Self { block, start, end }
    }
}
