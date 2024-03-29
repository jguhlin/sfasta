//! SeqLocs handle the location pointers for sequences, masks, IDs, etc...
//! Each SeqLoc will point to a start and a length of Loc objects for a given sequence

// TODO! Need tests...
// TODO: When data gets too large, pre-emptively compress it into memory (such as nt db, >200Gb).
// TODO: Flatten seqlocs into a single vec, then use ordinals to find appropritate ones
// TODO: Can convert this to use ByteBlockStore?

use std::io::{Read, Seek, SeekFrom, Write};

use crate::compression::*;

/*
Should flatten and have it stored as:
SeqLoc Index:
(Option<(u64, u32)>, Option<(u32, u32)>, Option(<u64, u32>), Option<(u64, u8)>, Option<(u64, u8)>
(Seq Start, # of Locs, Masking Start, # of Masking Locs, Scores Start, # of Scores Locs, Headers Start, # of Header Locs, IDs Start, # of ID Locs)

Then SeqLoc blocks are flattened version

#[derive(Debug, Clone, bincode::Encode, bincode::Decode, Default, PartialEq, Eq, Hash)]
pub struct SeqLoc {very
    pub sequence: Option<Vec<Loc>>,
    pub masking: Option<(u32, u32)>,
    pub scores: Option<Vec<Loc>>,
    pub headers: Option<Vec<Loc>>,
    pub ids: Option<Vec<Loc>>,
} */

/// Handles access to SeqLocs
#[derive(Clone)]
pub struct SeqLocs {
    location: u64,
    block_index_pos: u64,
    seqlocs_chunks_position: u64,
    seqlocs_chunks_offsets: Option<Vec<u32>>,
    block_locations: Option<Vec<u64>>,
    chunk_size: u32,
    seqlocs_chunk_size: u64,
    pub index: Option<Vec<SeqLoc>>,
    pub data: Option<Vec<Loc>>,
    total_locs: usize,
    pub total_seqlocs: usize,
    cache: Option<(u32, Vec<Loc>)>,
    decompression_buffer: Option<Vec<u8>>,
    compressed_seq_buffer: Option<Vec<u8>>,
    preloaded: bool,
}

impl std::ops::Index<usize> for SeqLocs {
    type Output = Loc;

    fn index(&self, index: usize) -> &Self::Output {
        &self.data.as_ref().unwrap()[index]
    }
}

impl Default for SeqLocs {
    fn default() -> Self {
        SeqLocs {
            location: 0,
            block_index_pos: 0,
            block_locations: None,
            chunk_size: 8 * 1024,
            seqlocs_chunk_size: 8 * 1024,
            seqlocs_chunks_offsets: None,
            index: None,
            total_locs: 0,
            total_seqlocs: 0,
            data: None,
            cache: None,
            decompression_buffer: None,
            compressed_seq_buffer: None,
            preloaded: false,
            seqlocs_chunks_position: 0,
        }
    }
}

impl SeqLocs {
    /// Create a new SeqLocs object
    pub fn new() -> Self {
        SeqLocs::default()
    }

    /// Get locs for a given seqloc (as defined by start and length)
    pub fn get_locs<R>(&mut self, mut in_buf: &mut R, start: usize, length: usize) -> Vec<Loc>
    where
        R: Read + Seek,
    {
        if self.data.is_some() {
            self.data.as_ref().unwrap()[start..start + length].to_vec()
        } else {
            let chunk_size = self.chunk_size as usize;
            let start_block = start / self.chunk_size as usize;
            let end_block = (start + length) / self.chunk_size as usize;

            let mut locs = Vec::with_capacity(length);
            if start_block == end_block {
                let block = self.get_block(&mut in_buf, start_block as u32);
                locs.extend_from_slice(&block[start % chunk_size..start % chunk_size + length]);
            } else {
                for block in start_block..=end_block {
                    let block_locs = self.get_block(&mut in_buf, block as u32);
                    if start_block == block {
                        locs.extend_from_slice(&block_locs[start % chunk_size..]);
                    } else if end_block == block {
                        locs.extend_from_slice(&block_locs[..(start + length) % chunk_size]);
                    } else {
                        locs.extend_from_slice(block_locs);
                    }
                }
            }
            locs
        }
    }

    /// Get locs for a given seqloc (as defined by start and length)
    pub fn get_locs_loaded(&self, start: usize, length: usize) -> Vec<Loc> {
        if self.data.is_some() {
            self.data.as_ref().unwrap()[start..start + length].to_vec()
        } else {
            panic!("Cannot get locs from SeqLocs that is not loaded");
        }
    }

    /// Get total number of SeqLocs
    pub fn index_len(&self) -> usize {
        self.total_seqlocs
    }

    /// Only for creation. Add a SeqLoc to the SeqLocs object
    pub fn add_to_index(&mut self, seqloc: SeqLoc) {
        if self.index.is_none() {
            self.index = Some(Vec::new());
        }
        self.index.as_mut().unwrap().push(seqloc);
        self.total_seqlocs += 1;
    }

    /// Only used during creation. Add a Loc to the index
    /// Returns the start index and length of the Locs
    pub fn add_locs(&mut self, locs: &[Loc]) -> (u64, u32) {
        if self.data.is_none() {
            self.data = Some(Vec::with_capacity(1024 * 256));
        }

        let start = self.total_locs;
        let len = locs.len();
        self.total_locs += len;
        self.data.as_mut().unwrap().extend_from_slice(locs);
        (start as u64, len as u32)
    }

    /// Prefetch the SeqLocs index into memory. Speeds up successive access, but can be a hefty one-time cost for large files.
    pub fn prefetch<R>(&mut self, mut in_buf: &mut R)
    where
        R: Read + Seek,
    {
        self.get_all_seqlocs(&mut in_buf)
            .expect("Unable to Prefetch All SeqLocs");

        let mut data = Vec::with_capacity(self.total_locs);
        log::info!(
            "Total Loc Blocks: {}",
            self.block_locations.as_ref().unwrap().len()
        );

        let mut in_buf = std::io::BufReader::with_capacity(512 * 1024, in_buf);

        for i in 0..self.block_locations.as_ref().unwrap().len() {
            let locs = self.get_block_uncached(&mut in_buf, i as u32);
            data.extend(locs);
        }

        self.data = Some(data);
        self.preloaded = true;

        log::info!("Prefetched {} seqlocs", self.total_locs);
    }

    /*pub fn with_data(index: Vec<SeqLoc>, data: Vec<Loc>) -> Self {
        SeqLocs {
            location: 0,
            block_index_pos: 0,
            block_locations: None,
            seqlocs_chunks_offsets: None,
            chunk_size: 256 * 1024,
            seqlocs_chunk_size: 1024,
            total_locs: data.len(),
            total_seqlocs: index.len(),
            index: Some(index),
            data: Some(data),
            cache: None,
            decompression_buffer: None,
            compressed_seq_buffer: None,
            zstd_decompressor: None,
            preloaded: true,
            seqlocs_chunks_offsets_position: 0,
            seqlocs_chunks_position: 0,
        }
    } */

    /// Set the location u64 of the SeqLocs object
    pub fn with_location(mut self, location: u64) -> Self {
        self.location = location;
        self
    }

    pub fn with_block_index_pos(mut self, block_index_pos: u64) -> Self {
        self.block_index_pos = block_index_pos;
        self
    }

    pub fn with_chunk_size(mut self, chunk_size: u32) -> Self {
        self.chunk_size = chunk_size;
        self
    }

    /// Write a SeqLocs object to a file (buffer)
    pub fn write_to_buffer<W>(&mut self, mut out_buf: &mut W) -> u64
    where
        W: Write + Seek,
    {
        let bincode_config = bincode::config::standard().with_variable_int_encoding();

        if self.data.is_none() {
            panic!("Unable to write SeqLocs as there are none");
        }

        let starting_pos = out_buf.stream_position().unwrap();

        let locs = self.data.take().unwrap();
        let total_locs = locs.len() as u64;

        let seq_locs = self.index.take().unwrap();
        let total_seq_locs = seq_locs.len() as u64;

        let seqlocs_location = out_buf
            .stream_position()
            .expect("Unable to work with seek API");

        let mut seqlocs_chunks_position = 0u64;
        let mut seqlocs_chunks_offsets_position = 0u64;

        let header = (
            self.chunk_size,
            self.seqlocs_chunk_size,
            self.block_index_pos,
            total_seq_locs,
            total_locs,
            seqlocs_chunks_position,         // Seqlocs Chunks Position
            seqlocs_chunks_offsets_position, // Seqlocs Chunks Offsets
        );

        // Write out header
        bincode::encode_into_std_write(header, &mut out_buf, bincode_config)
            .expect("Unable to write out chunk size");

        seqlocs_chunks_position = out_buf
            .stream_position()
            .expect("Unable to work with seek API");

        let mut seqlocs_chunk_offset: Vec<u32> =
            Vec::with_capacity((seq_locs.len() / self.seqlocs_chunk_size as usize) + 1);

        // TODO: Compression level here should be customizable, but also should be a fast one...
        let mut compressor = zstd_encoder(3, &None);

        let start = out_buf.stream_position().unwrap();

        // FORMAT: Write SeqLocs (Specified where to find the [Locs] for each datatype)
        let mut current_offset = 0;
        for chunk in seq_locs.chunks(self.seqlocs_chunk_size as usize) {
            seqlocs_chunk_offset.push(current_offset);
            let as_bytes = bincode::encode_to_vec(chunk, bincode_config).unwrap();
            let compressed = compressor.compress(&as_bytes).unwrap();

            // Write out the uncompressed length as u32
            match bincode::encode_into_std_write(
                as_bytes.len() as u32,
                &mut out_buf,
                bincode_config,
            ) {
                Ok(x) => current_offset += x as u32,
                Err(e) => {
                    panic!("Unable to write out seqlocs chunk size: {e}");
                }
            }

            // Write out the compressed SeqLoc chunk
            match bincode::encode_into_std_write(&compressed, &mut out_buf, bincode_config) {
                Ok(x) => {
                    current_offset += x as u32;
                }
                Err(e) => {
                    panic!("Unable to write out seqlocs chunk: {e}");
                }
            }
        }

        let end = out_buf.stream_position().unwrap();
        log::debug!(
            "Wrote {} seqlocs chunks in {} bytes",
            seqlocs_chunk_offset.len(),
            end - start
        );

        seqlocs_chunks_offsets_position = out_buf
            .stream_position()
            .expect("Unable to work with seek API");

        // Write out SeqLocs Chunk Offsets
        let data = bincode::encode_to_vec(&seqlocs_chunk_offset, bincode_config)
            .expect("Unable to write out seqlocs chunk offsets");

        match bincode::encode_into_std_write(data.len() as u32, &mut out_buf, bincode_config) {
            Ok(_) => (),
            Err(e) => {
                panic!("Unable to write out seqlocs chunk offsets size: {e}");
            }
        }

        let compressed = compressor.compress(&data).unwrap();
        match bincode::encode_into_std_write(compressed, &mut out_buf, bincode_config) {
            Ok(_) => (),
            Err(e) => {
                panic!("Unable to write out seqlocs chunk offsets size: {e}");
            }
        }

        // FORMAT: Write sequence location blocks

        let mut block_locations: Vec<u64> =
            Vec::with_capacity((seq_locs.len() / self.chunk_size as usize) + 1);

        let mut bincoded: Vec<u8>;

        let start = out_buf.stream_position().unwrap();
        let mut compressed_buf: Vec<u8> = Vec::with_capacity(128 * 1024);

        for chunk in locs.chunks(self.chunk_size as usize) {
            block_locations.push(
                out_buf
                    .stream_position()
                    .expect("Unable to work with seek API")
                    - starting_pos,
            );

            // TODO: Also should be customizable compression level
            let mut compressor =
                zstd::bulk::Compressor::new(3).expect("Unable to create zstd encoder");
            compressor.include_magicbytes(false).unwrap();
            compressor.long_distance_matching(true).unwrap();

            bincoded = bincode::encode_to_vec(chunk, bincode_config)
                .expect("Unable to bincode locs into compressor");

            compressor
                .compress_to_buffer(&bincoded, &mut compressed_buf)
                .unwrap();

            bincode::encode_into_std_write(&compressed_buf[..], &mut out_buf, bincode_config)
                .expect("Unable to write Sequence Blocks to file");

            bincoded.clear();
            compressed_buf.clear();
        }

        let end = out_buf.stream_position().unwrap();
        log::debug!(
            "Wrote {} Loc blocks in {} bytes",
            block_locations.len(),
            end - start
        );

        self.block_index_pos = out_buf
            .stream_position()
            .expect("Unable to work with seek API");

        // This needs to be in small chunks and then with offsets for values (instead of absolute locations)
        let bincoded = bincode::encode_to_vec(&block_locations, bincode_config)
            .expect("Unable to bincode locs into compressor");

        // TODO: Also should be customizable compression level
        let mut compressor = zstd::stream::Encoder::new(Vec::with_capacity(2 * 1024 * 1024), 3)
            .expect("Unable to create zstd encoder");
        compressor.include_magicbytes(false).unwrap();
        compressor.long_distance_matching(true).unwrap();

        compressor.write_all(&bincoded).unwrap();
        let compressed = compressor.finish().unwrap();

        bincode::encode_into_std_write(compressed, &mut out_buf, bincode_config)
            .expect("Unable to write Sequence Blocks to file");

        log::debug!("Wrote block index");

        self.block_locations = Some(block_locations);

        let end = out_buf
            .stream_position()
            .expect("Unable to work with seek API");

        out_buf.seek(SeekFrom::Start(starting_pos)).unwrap();

        // let mut header = (self.chunk_size, self.block_index_pos, total_seq_locs, total_locs);
        let header = (
            self.chunk_size,
            self.seqlocs_chunk_size,
            self.block_index_pos,
            total_seq_locs,
            total_locs,
            seqlocs_chunks_position,         // Seqlocs Chunks Position
            seqlocs_chunks_offsets_position, // Seqlocs Chunks Offsets
        );

        // Write out updated header...
        bincode::encode_into_std_write(header, &mut out_buf, bincode_config)
            .expect("Unable to write out chunk size");

        // Go back to the end of the file...
        out_buf.seek(SeekFrom::Start(end)).unwrap();

        log::debug!("Wrote header - SeqLoc Write to buffer complete");
        seqlocs_location
    }

    /// Open SeqLocs object from a file (buffer)
    pub fn from_buffer<R>(mut in_buf: &mut R, pos: u64) -> Result<Self, String>
    where
        R: Read + Seek,
    {
        let current_pos = in_buf
            .stream_position()
            .expect("Unable to work with seek API");

        // Really the remaining length
        let buffer_len = in_buf
            .seek(SeekFrom::End(0))
            .expect("Unable to work with seek API");

        // Reset position
        in_buf
            .seek(SeekFrom::Start(current_pos))
            .expect("Unable to work with seek API");

        let mut decompressor = zstd::bulk::Decompressor::new().unwrap();
        decompressor.include_magicbytes(false).unwrap();

        let bincode_config = bincode::config::standard()
            .with_variable_int_encoding()
            .with_limit::<{ 4 * 1024 }>();

        in_buf
            .seek(SeekFrom::Start(pos))
            .expect("Unable to work with seek API");

        let header: (u32, u64, u64, u64, u64, u64, u64) =
            match bincode::decode_from_std_read(&mut in_buf, bincode_config) {
                Ok(h) => h,
                Err(e) => {
                    return Err(format!("Unable to read header: {e}"));
                }
            };

        let (
            chunk_size,
            seqlocs_chunk_size,
            block_index_pos,
            total_seq_locs,
            total_locs,
            seqlocs_chunks_position,
            seqlocs_chunks_offsets_position,
        ) = header;

        let bincode_config = bincode::config::standard()
            .with_variable_int_encoding()
            .with_limit::<{ 8 * 1024 * 1024 }>();

        log::info!("Decompressing SeqLoc Chunk Offsets");
        in_buf
            .seek(SeekFrom::Start(seqlocs_chunks_offsets_position))
            .unwrap();

        let data_len: u32 = match bincode::decode_from_std_read(&mut in_buf, bincode_config) {
            Ok(l) => l,
            Err(e) => {
                return Err(format!("Unable to read seqlocs chunk offsets size: {e}"));
            }
        };

        if data_len == 0 {
            return Err(format!("SeqLocs Data Len is erroneously 0"));
        }

        if data_len == std::u32::MAX {
            return Err(format!("SeqLocs Data Len is erroneously std::u32::MAX"));
        }

        if data_len > buffer_len as u32 {
            return Err(format!(
                "SeqLocs Data Len is greater than buffer len: {} > {}",
                data_len, buffer_len
            ));
        }

        let offsets_compressed: Vec<u8> =
            match bincode::decode_from_std_read(&mut in_buf, bincode_config) {
                Ok(x) => x,
                Err(e) => {
                    return Err(format!("Unable to read seqlocs chunk offsets: {e}"));
                }
            };

        let seqlocs_chunks_offsets_raw: Vec<u8> =
            match decompressor.decompress(&offsets_compressed, data_len as usize) {
                Ok(x) => x,
                Err(e) => {
                    return Err(format!("Unable to decompress seqlocs chunk offsets: {e}"));
                }
            };

        let seqlocs_chunks_offsets: Vec<u32> =
            match bincode::decode_from_slice(&seqlocs_chunks_offsets_raw, bincode_config) {
                Ok(x) => x.0,
                Err(e) => {
                    return Err(format!("Unable to read seqlocs chunk offsets: {e}"));
                }
            };

        log::info!("Decompressing Compressed Block Locations");
        in_buf.seek(SeekFrom::Start(block_index_pos)).unwrap();

        let compressed_block_locations: Vec<u8> =
            match bincode::decode_from_std_read(&mut in_buf, bincode_config) {
                Ok(c) => c,
                Err(e) => {
                    return Err(format!("Unable to read block index pos: {e}"));
                }
            };

        let mut decompressor =
            zstd::stream::read::Decoder::new(&compressed_block_locations[..]).unwrap();
        decompressor.include_magicbytes(false).unwrap();

        let mut decompressed = Vec::new();
        match decompressor.read_to_end(&mut decompressed) {
            Ok(_) => (),
            Err(e) => {
                return Err(format!("Unable to decompress block locations: {e}"));
            }
        }

        let block_locations: Vec<u64> =
            match bincode::decode_from_std_read(&mut decompressed.as_slice(), bincode_config) {
                Ok(c) => c,
                Err(e) => {
                    return Err(format!("Unable to read block index pos: {e}"));
                }
            };

        Ok(SeqLocs {
            location: pos,
            block_index_pos,
            block_locations: Some(block_locations),
            chunk_size,
            seqlocs_chunk_size,
            data: None,  // Don't decompress anything until requested...
            index: None, // Some(seq_locs),
            cache: None,
            total_locs: total_locs as usize,
            total_seqlocs: total_seq_locs as usize,
            decompression_buffer: None,
            compressed_seq_buffer: None,
            preloaded: false,
            seqlocs_chunks_position,
            seqlocs_chunks_offsets: Some(seqlocs_chunks_offsets),
        })
    }

    pub fn is_initialized(&self) -> bool {
        self.block_locations.is_some()
    }

    /// Get a particular block of Locs from the store
    pub fn get_block<R>(&mut self, in_buf: &mut R, block: u32) -> &[Loc]
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

            self.cache = Some((block, seqlocs));

            &self.cache.as_ref().unwrap().1
        }
    }

    /// Get a particular block of Locs from the store, without using a cache
    fn get_block_uncached<R>(&mut self, mut in_buf: &mut R, block: u32) -> Vec<Loc>
    where
        R: Read + Seek,
    {
        let block_locations = self.block_locations.as_ref().unwrap();
        let block_location = block_locations[block as usize];
        in_buf
            .seek(SeekFrom::Start(self.location + block_location))
            .unwrap();

        let bincode_config = bincode::config::standard()
            .with_variable_int_encoding()
            .with_limit::<{ 2 * 1024 * 1024 }>();

        if self.compressed_seq_buffer.is_none() {
            self.compressed_seq_buffer = Some(Vec::with_capacity(256 * 1024));
        } else {
            self.compressed_seq_buffer.as_mut().unwrap().clear();
        }

        let compressed_block = self.compressed_seq_buffer.as_mut().unwrap();

        *compressed_block = bincode::decode_from_std_read(&mut in_buf, bincode_config)
            .expect("Unable to read block");

        let mut zstd_decompressor = zstd_decompressor(None);

        if self.decompression_buffer.is_none() {
            self.decompression_buffer = Some(Vec::with_capacity(512 * 1024));
        }

        let decompressed = self.decompression_buffer.as_mut().unwrap();

        zstd_decompressor
            .decompress_to_buffer(compressed_block, decompressed)
            .unwrap();

        // decompressor.read_to_end(decompressed).unwrap();

        let locs: Vec<Loc> =
            bincode::decode_from_std_read(&mut decompressed.as_slice(), bincode_config)
                .expect("Unable to read block");
        locs
    }

    /// Load up all SeqLocs from a file
    pub fn get_all_seqlocs<R>(
        &mut self,
        in_buf: &mut R,
    ) -> Result<Option<&Vec<SeqLoc>>, &'static str>
    where
        R: Read + Seek,
    {
        if self.index.is_none() {
            // Because this is sequential reading, we use a BufReader
            let mut in_buf = std::io::BufReader::new(in_buf);

            let mut decompressor = zstd::bulk::Decompressor::new().unwrap();
            decompressor
                .set_parameter(zstd::stream::raw::DParameter::ForceIgnoreChecksum(true))
                .unwrap();
            decompressor.include_magicbytes(false).unwrap();

            log::info!("Prefetching SeqLocs");

            let bincode_config = bincode::config::standard().with_variable_int_encoding();

            in_buf
                .seek(SeekFrom::Start(self.seqlocs_chunks_position))
                .unwrap();
            let mut seqlocs = Vec::with_capacity(self.total_seqlocs);
            let mut length: u32;

            // Go to the start, then just read sequentially..
            in_buf
                .seek(SeekFrom::Start(
                    self.seqlocs_chunks_position
                        + self.seqlocs_chunks_offsets.as_ref().unwrap()[0] as u64,
                ))
                .unwrap();

            log::debug!(
                "Reading {} chunks of SeqLocs",
                self.seqlocs_chunks_offsets.as_ref().unwrap().len()
            );
            let mut seqlocs_chunk_raw = Vec::with_capacity(64 * 1024);
            let mut seqlocs_chunk_compressed: Vec<u8> = Vec::with_capacity(64 * 1024);
            let mut seqlocs_chunk: Vec<SeqLoc>;
            // TODO: Zero copy deserialization possible here?
            for _offset in self.seqlocs_chunks_offsets.as_ref().unwrap().iter() {
                length = bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();

                seqlocs_chunk_raw.clear();
                seqlocs_chunk_raw.reserve(length as usize);

                seqlocs_chunk_compressed.clear();
                seqlocs_chunk_compressed =
                    bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();

                decompressor
                    .decompress_to_buffer(&seqlocs_chunk_compressed, &mut seqlocs_chunk_raw)
                    .unwrap();

                seqlocs_chunk = bincode::decode_from_slice(&seqlocs_chunk_raw, bincode_config)
                    .unwrap()
                    .0;
                seqlocs.append(&mut seqlocs_chunk);
            }
            self.index = Some(seqlocs);
        }

        log::debug!("Finished");

        return Ok(self.index.as_ref());
    }

    /// Get a particular SeqLoc from the store
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

        if self.index.is_some() && index as usize >= self.index.as_ref().unwrap().len() {
            return Err("Index out of bounds");
        }

        if self.index.is_some() {
            Ok(Some(self.index.as_ref().unwrap()[index as usize].clone()))
        } else {
            let mut decompressor = zstd::bulk::Decompressor::new().unwrap();
            decompressor.include_magicbytes(false).unwrap();

            let bincode_config = bincode::config::standard().with_variable_int_encoding();

            in_buf
                .seek(SeekFrom::Start(self.seqlocs_chunks_position))
                .unwrap();

            log::debug!("Seeking to position: {}", self.seqlocs_chunks_position);

            let chunk = index as usize / self.seqlocs_chunk_size as usize;
            let offset = index as usize % self.seqlocs_chunk_size as usize;
            let byte_offset = self.seqlocs_chunks_offsets.as_ref().unwrap()[chunk];

            log::debug!("Seeking additional byte offset: {}", byte_offset);
            in_buf.seek(SeekFrom::Current(byte_offset as i64)).unwrap();
            let length: u32 = bincode::decode_from_std_read(in_buf, bincode_config).unwrap();
            log::debug!("Length: {}", length);
            let seqlocs_chunk_raw: Vec<u8> =
                bincode::decode_from_std_read(in_buf, bincode_config).unwrap();
            log::debug!("Got SeqLocs Chunk Raw: {}", seqlocs_chunk_raw.len());
            let seqlocs_chunk_compressed: Vec<u8> = decompressor
                .decompress(&seqlocs_chunk_raw, length as usize)
                .unwrap();
            log::debug!("Got SeqLocs Chunk Compressed");
            let seqlocs_chunk: Vec<SeqLoc> =
                bincode::decode_from_slice(&seqlocs_chunk_compressed, bincode_config)
                    .unwrap()
                    .0;
            log::debug!("Got SeqLocs Chunk");
            let seqloc = seqlocs_chunk[offset].clone();

            log::debug!("Got SeqLoc");
            Ok(Some(seqloc))
        }
    }
}

#[derive(Clone, Debug, bincode::Encode, bincode::Decode, Default, PartialEq, Eq)]
pub struct SeqLoc {
    pub sequence: Option<(u64, u32)>,
    pub masking: Option<(u64, u32)>,
    pub scores: Option<(u16, u16)>,
    pub headers: Option<(u64, u8)>,
    pub ids: Option<(u64, u8)>,
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

    #[allow(clippy::len_without_is_empty)]
    pub fn len<R>(&self, seqlocs: &mut SeqLocs, mut in_buf: &mut R, block_size: u32) -> usize
    where
        R: Read + Seek,
    {
        let locs = seqlocs.get_locs(
            &mut in_buf,
            self.sequence.unwrap().0 as usize,
            self.sequence.unwrap().1 as usize,
        );
        locs.into_iter().map(|loc| loc.len as usize).sum()
    }

    pub fn len_loaded(&self, seqlocs: &SeqLocs, block_size: u32) -> usize {
        let locs = seqlocs.get_locs_loaded(
            self.sequence.unwrap().0 as usize,
            self.sequence.unwrap().1 as usize,
        );
        locs.into_iter().map(|loc| loc.len as usize).sum()
    }

    // Convert Vec of Locs to the ranges of the sequence...
    pub fn seq_location_splits(sequence: &[Loc]) -> Vec<std::ops::Range<u32>> {
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
    pub fn seq_slice<R>(
        &self,
        seqlocs: &mut SeqLocs,
        mut in_buf: &mut R,
        block_size: u32,
        range: std::ops::Range<u32>,
    ) -> Vec<Loc>
    where
        R: Read + Seek,
    {
        let mut new_locs = Vec::new();

        let locs = seqlocs.get_locs(
            &mut in_buf,
            self.sequence.as_ref().unwrap().0 as usize,
            self.sequence.as_ref().unwrap().1 as usize,
        );

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
                // For example, Loc is 1500..2000 (length 500 in this Loc) and the range we want is 450..550 (so 1950..2000 from this loc, and another 100 from the next loc)
                let start = range.start.saturating_sub(split.start);
                let end = block_size; // range.end.saturating_sub(split.start);
                new_locs.push(locs[i].slice(start..end));
            } else if split.contains(&end) {
                // Loc contains the end of the range...
                // For example, Loc is 1500..2000 (length 500 in this Loc, starting at 1000) and the range we want is 900..1200 (so 1500..1700 from this loc)
                let start = range.start.saturating_sub(split.start);
                let end = range.end.saturating_sub(split.start);
                new_locs.push(locs[i].slice(start..end));
                break; // We are done if it contains the end of the range...
            } else if split.start > range.start && split.end < range.end {
                // Loc contains the entire range...
                // For example, Loc is 1500..2000 (length 500 in the Loc) and the range we want is 450..550 (so 1500..1550 from this loc, and another 100 from the previous loc)
                new_locs.push(locs[i].clone());
            } else {
                // Loc does not contain the range...
                // For example, Loc is 1500..2000 (length 500 in the Loc) and the range we want is 250..350 (so 1750..1800 from this loc)
            }
        }

        new_locs
    }
}

#[derive(Debug, Clone, bincode::Encode, bincode::Decode, PartialEq, Eq, Hash)]
pub struct Loc {
    pub block: u32,
    pub start: u32,
    pub len: u32,
}

impl Loc {
    #[inline]
    pub fn new(block: u32, start: u32, len: u32) -> Self {
        Self { block, start, len }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Slice a loc
    /// Offset from the total sequence should be calculated before this step
    /// But this handles calculating from inside the loc itself...
    // TODO: Implement for RangeInclusive
    #[inline]
    pub fn slice(&self, range: std::ops::Range<u32>) -> Loc {
        Loc {
            block: self.block,
            start: std::cmp::max(self.start.saturating_add(range.start), self.start),
            len: range.end.saturating_sub(range.start),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_seqloc_slice() {
        let mut seqlocs = SeqLocs::default();
        let mut seqloc = SeqLoc::new();

        let mut dummy_buffer = std::io::Cursor::new(vec![0; 1024]);

        seqloc.sequence = Some(seqlocs.add_locs(&vec![
            Loc::new(0, 0, 10),
            Loc::new(1, 0, 10),
            Loc::new(2, 0, 10),
            Loc::new(3, 0, 10),
            Loc::new(4, 0, 10),
        ]));
        let slice = seqloc.seq_slice(&mut seqlocs, &mut dummy_buffer, 10, 0..10);
        assert_eq!(slice, vec![Loc::new(0, 0, 10)]);
        let slice = seqloc.seq_slice(&mut seqlocs, &mut dummy_buffer, 10, 5..7);
        assert_eq!(slice, vec![Loc::new(0, 5, 2)]);
        let slice = seqloc.seq_slice(&mut seqlocs, &mut dummy_buffer, 10, 15..17);
        assert_eq!(slice, vec![Loc::new(1, 5, 2)]);
        let slice = seqloc.seq_slice(&mut seqlocs, &mut dummy_buffer, 10, 10..20);
        assert_eq!(slice, vec![Loc::new(1, 0, 10)]);
        let slice = seqloc.seq_slice(&mut seqlocs, &mut dummy_buffer, 10, 20..30);
        assert_eq!(slice, vec![Loc::new(2, 0, 10)]);
        let slice = seqloc.seq_slice(&mut seqlocs, &mut dummy_buffer, 10, 15..35);
        assert_eq!(
            slice,
            vec![Loc::new(1, 5, 5), Loc::new(2, 0, 10), Loc::new(3, 0, 5)]
        );
        let slice = seqloc.seq_slice(&mut seqlocs, &mut dummy_buffer, 10, 5..9);
        assert_eq!(slice, vec![Loc::new(0, 5, 4)]);
        let block_size = 262144;
        seqloc.sequence = Some(seqlocs.add_locs(&vec![
            Loc::new(3097440, 261735, 262144 - 261735),
            Loc::new(3097441, 0, 1274),
        ]));

        //                                  x 261735 ----------> 262144  (262144 - 261735) = 409
        //     -------------------------------------------------
        //     <----- 1274                                                                 = 1274
        //     -------------------------------------------------
        //     So total is 409 + 1274 = 1683
        //
        //     We want 104567 to 104840 -- how?

        let slice = seqloc.seq_slice(&mut seqlocs, &mut dummy_buffer, block_size, 0..20);
        assert_eq!(slice, vec![Loc::new(3097440, 261735, 20)]);

        seqloc.sequence = Some(seqlocs.add_locs(&vec![
            Loc::new(1652696, 260695, 262144 - 260695),
            Loc::new(1652697, 0, 28424),
        ]));

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

        let slice = seqloc.seq_slice(&mut seqlocs, &mut dummy_buffer, block_size, 2679..2952);
        assert_eq!(slice, vec![Loc::new(1652697, 1230, 2952 - 2679)]);
    }
}
