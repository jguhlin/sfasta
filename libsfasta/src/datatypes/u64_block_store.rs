use std::io::{Read, Seek, SeekFrom, Write};

use crate::datatypes::{zstd_encoder, CompressionType};

// TODO: WIP:
// TODO: Read for testing, and then implementation

// Originally was using bitpacking, by Converting [u64] to [u32] of double length, and decompressing that. But that was giving errors and was intractable without a specific datastructure to handle it.
// So now we are using zstd (look into zigzag, stream vbyte 64, etc, in the future)
// But want something to work now

// This encodes everything as zstd blocks
// So [u64] -> Multiple blocks of [u64] that are zstd compressed, with a block index

// This is a specialized variant of Bytes Block Store, but we don't need the entire Loc system, just the # of the block and the location in the block (ordinal)


pub struct U64BlockStore {
    location: u64,
    block_index_pos: u64,
    block_locations: Option<Vec<u64>>,
    block_size: u64,
    pub data: Option<Vec<u64>>,
    pub compression_type: CompressionType,
    cache: Option<(u32, Vec<u64>)>,
    compressed_blocks: Option<Vec<u8>>,
    compressed_block_lens: Option<Vec<usize>>,
    counter: usize,
}

impl Default for U64BlockStore {
    fn default() -> Self {
        U64BlockStore {
            location: 0,
            block_index_pos: 0,
            block_locations: None,
            block_size: 2048, // 8 bytes is a u64, 2048 is the number of u64s in a block
            data: None,
            compression_type: CompressionType::ZSTD,
            cache: None,
            compressed_blocks: None,
            compressed_block_lens: None,
            counter: 0,
        }
    }
}

impl U64BlockStore {
    pub fn with_block_size(mut self, block_size: usize) -> Self {
        self.block_size = block_size as u64;
        self
    }

    fn compress_block(&mut self) {
        let mut compressor = zstd_encoder(3, None);

        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        if self.compressed_blocks.is_none() {
            self.compressed_block_lens = Some(Vec::new());
            self.compressed_blocks = Some(Vec::new());
        }

        #[cfg(test)]
        let mut compressed = Vec::with_capacity(8192);

        #[cfg(not(test))]
        let mut compressed = Vec::with_capacity(self.block_size as usize);

        let at = std::cmp::min(self.block_size as usize, self.data.as_mut().unwrap().len());

        let mut block = self.data.as_mut().unwrap().split_off(at);
        block.reserve(self.block_size as usize);
        std::mem::swap(&mut block, self.data.as_mut().unwrap());

        let data = bincode::encode_to_vec(&block, bincode_config).unwrap();

        let compressed_size = compressor
            .compress_to_buffer(&data, &mut compressed)
            .unwrap();

        self.compressed_block_lens
            .as_mut()
            .unwrap()
            .push(compressed_size);

        self.compressed_blocks.as_mut().unwrap().extend(compressed);
    }

    pub fn add(&mut self, input: u64) -> usize {
        if self.data.is_none() {
            self.data = Some(Vec::with_capacity(self.block_size as usize));
        }

        while self.data.as_ref().unwrap().len() > self.block_size as usize {
            self.compress_block();
        }

        let data = self.data.as_mut().unwrap();
        data.push(input);

        self.counter += 1;
        self.counter
    }

    pub fn emit_blocks(&mut self) -> Vec<&[u8]> {
        while self.data.as_ref().unwrap().len() > 0 {
            self.compress_block();
        }

        let data = self.compressed_blocks.as_ref().unwrap();
        let mut blocks = Vec::new();

        let mut start = 0;
        for len in self.compressed_block_lens.as_ref().unwrap() {
            blocks.push(&data[start..start + len]);
            start += len;
        }

        blocks
    }

    pub fn write_to_buffer<W>(&mut self, mut out_buf: &mut W) -> Option<u64>
    where
        W: Write + Seek,
    {
        self.data.as_ref()?;

        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        let mut block_locations_pos: u64 = 0;

        let starting_pos = out_buf.seek(SeekFrom::Current(0)).unwrap();
        // TODO: This is a lie, only zstd is supported as of right now...
        bincode::encode_into_std_write(self.compression_type, &mut out_buf, bincode_config)
            .unwrap();
        bincode::encode_into_std_write(block_locations_pos, &mut out_buf, bincode_config).unwrap();
        bincode::encode_into_std_write(self.block_size, &mut out_buf, bincode_config).unwrap();

        let mut block_locations = Vec::new();

        let compressed_blocks = self.emit_blocks();

        for compressed_block in compressed_blocks {
            let block_start = out_buf.seek(SeekFrom::Current(0)).unwrap();
            bincode::encode_into_std_write(&compressed_block, &mut out_buf, bincode_config)
                .unwrap();
            block_locations.push(block_start);
        }

        block_locations_pos = out_buf.seek(SeekFrom::Current(0)).unwrap();
        bincode::encode_into_std_write(&block_locations, &mut out_buf, bincode_config).unwrap();
        self.block_locations = Some(block_locations);

        let end = out_buf.seek(SeekFrom::Current(0)).unwrap();
        out_buf.seek(SeekFrom::Start(starting_pos)).unwrap();
        bincode::encode_into_std_write(self.compression_type, &mut out_buf, bincode_config)
            .unwrap();
        bincode::encode_into_std_write(block_locations_pos, &mut out_buf, bincode_config).unwrap();
        bincode::encode_into_std_write(self.block_size, &mut out_buf, bincode_config).unwrap();

        // Back to the end so we don't interfere with anything...
        out_buf.seek(SeekFrom::Start(end)).unwrap();

        Some(starting_pos)
    }

    pub fn from_buffer<R>(mut in_buf: &mut R, starting_pos: u64) -> Result<Self, String>
    where
        R: Read + Seek,
    {
        let bincode_config = bincode::config::standard()
            .with_fixed_int_encoding()
            .with_limit::<{ 4 * 1024 * 1024 }>();

        let mut store = U64BlockStore::default();

        in_buf.seek(SeekFrom::Start(starting_pos)).unwrap();
        (
            store.compression_type,
            store.block_index_pos,
            store.block_size,
        ) = match bincode::decode_from_std_read(&mut in_buf, bincode_config) {
            Ok(x) => x,
            Err(e) => return Err(format!("Error decoding block store: {}", e)),
        };

        store.location = starting_pos;

        in_buf.seek(SeekFrom::Start(store.block_index_pos)).unwrap();

        store.block_locations = Some(
            match bincode::decode_from_std_read(&mut in_buf, bincode_config) {
                Ok(x) => x,
                Err(e) => return Err(format!("Error decoding block locations: {}", e)),
            },
        );

        Ok(store)
    }

    pub fn prefetch<R>(&mut self, in_buf: &mut R)
    where
        R: Read + Seek,
    {
        let mut data = Vec::with_capacity(
            self.block_size as usize * self.block_locations.as_ref().unwrap().len(),
        );

        for i in 0..self.block_locations.as_ref().unwrap().len() {
            data.extend(self.get_block_uncached(in_buf, i as u32));
        }
        log::info!("Generic Block Store Prefetching done: {}", data.len());
        self.data = Some(data);
    }

    pub fn get_block<R>(&mut self, in_buf: &mut R, block: u32) -> Vec<u64>
    where
        R: Read + Seek,
    {
        if self.cache.is_some() && self.cache.as_ref().unwrap().0 == block {
            return self.cache.as_ref().unwrap().1.clone();
        } else {
            self.cache = Some((block, self.get_block_uncached(in_buf, block)));
            return self.cache.as_ref().unwrap().1.clone();
        }
    }

    pub fn get_block_uncached<R>(&mut self, mut in_buf: &mut R, block: u32) -> Vec<u64>
    where
        R: Read + Seek,
    {
        let bincode_config = bincode::config::standard()
            .with_fixed_int_encoding()
            .with_limit::<{ 128 * 1024 * 1024 }>();
        let block_locations = self.block_locations.as_ref().unwrap();

        let mut decompressor = zstd::bulk::Decompressor::new().unwrap();
        decompressor.include_magicbytes(false).unwrap();

        let block_location = block_locations[block as usize];
        in_buf.seek(SeekFrom::Start(block_location)).unwrap();
        let compressed_block: Vec<u8> =
            bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();

        let decompressed_block = decompressor
            .decompress(&compressed_block, 8 * self.block_size as usize)
            .unwrap();

        bincode::decode_from_slice(&decompressed_block, bincode_config)
            .unwrap()
            .0
    }

    pub fn get<R>(&mut self, in_buf: &mut R, x: usize) -> Vec<u64>
    where
        R: Read + Seek,
    {
        if self.data.is_some() {
            self.data.as_ref().unwrap()[x..x + self.block_size as usize].to_vec()
        } else {
            self.get_block(in_buf, (x % self.block_size as usize) as u32)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
}
