use std::io::{Read, Seek, SeekFrom, Write};

use crate::datatypes::{zstd_encoder, CompressionType, Loc};

#[derive(Clone)]
pub struct BytesBlockStore {
    location: u64,
    block_index_pos: u64,
    block_locations: Option<Vec<u64>>,
    block_size: usize,
    pub data: Option<Vec<u8>>, // Only used for writing...
    pub compression_type: CompressionType,
    cache: Option<(u32, Vec<u8>)>,
    compressed_blocks: Option<Vec<u8>>,
    compressed_block_lens: Option<Vec<usize>>,
}

impl Default for BytesBlockStore {
    fn default() -> Self {
        BytesBlockStore {
            location: 0,
            block_index_pos: 0,
            block_locations: None,
            block_size: 512 * 1024,
            data: None,
            compression_type: CompressionType::ZSTD,
            cache: None,
            compressed_blocks: None,
            compressed_block_lens: None,
        }
    }
}

impl BytesBlockStore {
    pub fn with_block_size(mut self, block_size: usize) -> Self {
        self.block_size = block_size;
        self
    }

    fn compress_block(&mut self) {
        let mut compressor = zstd_encoder(3, None);

        if self.compressed_blocks.is_none() {
            self.compressed_block_lens = Some(Vec::new());
            self.compressed_blocks = Some(Vec::new());
        }

        #[cfg(test)]
        let mut compressed = Vec::with_capacity(8192);

        #[cfg(not(test))]
        let mut compressed = Vec::with_capacity(self.block_size);

        let at = std::cmp::min(self.block_size, self.data.as_mut().unwrap().len());

        let mut block = self.data.as_mut().unwrap().split_off(at);
        block.reserve(self.block_size);
        std::mem::swap(&mut block, self.data.as_mut().unwrap());

        let compressed_size = compressor
            .compress_to_buffer(&block, &mut compressed)
            .unwrap();

        self.compressed_block_lens
            .as_mut()
            .unwrap()
            .push(compressed_size);

        self.compressed_blocks.as_mut().unwrap().extend(compressed);
    }

    // TODO: Brotli compress very large ID blocks in memory(or LZ4)? Such as NT...
    pub fn add<'b, I: IntoIterator<Item = &'b u8>>(&'b mut self, input: I) -> Vec<Loc> {
        if self.data.is_none() {
            self.data = Some(Vec::with_capacity(self.block_size));
        }

        while self.data.as_ref().unwrap().len() > self.block_size {
            self.compress_block();
        }

        let data = self.data.as_mut().unwrap();

        let mut start = data.len();
        data.extend(input);
        let end = data.len() - 1;

        let compressed_blocks_count = match self.compressed_block_lens.as_ref() {
            Some(v) => v.len(),
            None => 0,
        };

        let starting_block = (start / self.block_size) + compressed_blocks_count;
        let ending_block = (end / self.block_size) + compressed_blocks_count;

        let mut locs = Vec::new();

        for block in starting_block..=ending_block {
            let block_start = start % self.block_size;
            let block_end = if block == ending_block {
                end % self.block_size
            } else {
                self.block_size - 1
            };
            start = block_end + 1;
            locs.push(Loc::Loc(block as u32, block_start as u32, block_end as u32));
        }

        locs
    }

    pub fn emit_blocks(&mut self) -> Vec<&[u8]> {
        while !self.data.as_ref().unwrap().is_empty() {
            self.compress_block();
        }

        let data = self.compressed_blocks.as_ref().unwrap();
        let mut blocks = Vec::new();
        let _len = data.len();

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

        let starting_pos = out_buf.stream_position().unwrap();
        // TODO: This is a lie, only zstd is supported as of right now...
        bincode::encode_into_std_write(self.compression_type, &mut out_buf, bincode_config)
            .unwrap();
        bincode::encode_into_std_write(block_locations_pos, &mut out_buf, bincode_config).unwrap();
        bincode::encode_into_std_write(self.block_size, &mut out_buf, bincode_config).unwrap();

        let mut block_locations = Vec::new();

        // Emits compressed blocks
        let blocks = self.emit_blocks();

        for compressed_block in blocks {
            let block_start = out_buf.stream_position().unwrap();
            bincode::encode_into_std_write(compressed_block, &mut out_buf, bincode_config).unwrap();
            block_locations.push(block_start);
        }

        block_locations_pos = out_buf.stream_position().unwrap();

        let bincoded_block_locations_size =
            bincode::encode_to_vec(&block_locations, bincode_config).unwrap();

        let compressed_block_locations =
            zstd::bulk::compress(&bincoded_block_locations_size, -3).unwrap();

        bincode::encode_into_std_write(&compressed_block_locations, &mut out_buf, bincode_config)
            .unwrap();
        self.block_locations = Some(block_locations);

        let end = out_buf.stream_position().unwrap();
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
            .with_limit::<{ 64 * 1024 * 1024 }>();

        let mut store = BytesBlockStore::default();

        in_buf.seek(SeekFrom::Start(starting_pos)).unwrap();
        (
            store.compression_type,
            store.block_index_pos,
            store.block_size,
        ) = match bincode::decode_from_std_read(&mut in_buf, bincode_config) {
            Ok(x) => x,
            Err(e) => return Err(format!("Error decoding block store: {e}")),
        };

        store.location = starting_pos;

        in_buf.seek(SeekFrom::Start(store.block_index_pos)).unwrap();
        let compressed: Vec<u8> = match bincode::decode_from_std_read(&mut in_buf, bincode_config) {
            Ok(x) => x,
            Err(e) => return Err(format!("Error decoding block locations: {e}")),
        };

        let block_locations: Vec<u8> = zstd::stream::decode_all(&compressed[..]).unwrap();
        let block_locations: Vec<u64> =
            match bincode::decode_from_slice(&block_locations, bincode_config)
            {
                Ok(x) => x.0,
                Err(e) => return Err(format!("Error decoding block locations: {e}")),
            };
                

        store.block_locations = Some(block_locations);

        Ok(store)
    }

    pub fn prefetch<R>(&mut self, in_buf: &mut R)
    where
        R: Read + Seek,
    {
        let mut data =
            Vec::with_capacity(self.block_size * self.block_locations.as_ref().unwrap().len());

        for i in 0..self.block_locations.as_ref().unwrap().len() {
            data.extend(self.get_block_uncached(in_buf, i as u32));
        }
        log::info!("Generic Block Store Prefetching done: {}", data.len());
        self.data = Some(data);
    }

    pub fn get_block<R>(&mut self, in_buf: &mut R, block: u32) -> Vec<u8>
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

    pub fn get_block_uncached<R>(&mut self, mut in_buf: &mut R, block: u32) -> Vec<u8>
    where
        R: Read + Seek,
    {
        let bincode_config = bincode::config::standard()
            .with_fixed_int_encoding()
            .with_limit::<{ 128 * 1024 * 1024 }>(); // 128 MB is max limit TODO: Enforce elsewhere too

        let block_locations = self.block_locations.as_ref().unwrap();

        let mut decompressor = zstd::bulk::Decompressor::new().unwrap();
        decompressor.include_magicbytes(false).unwrap();

        let block_location = block_locations[block as usize];
        in_buf.seek(SeekFrom::Start(block_location)).unwrap();
        let compressed_block: Vec<u8> =
            bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();

        decompressor
            .decompress(&compressed_block, self.block_size)
            .unwrap()
    }

    pub fn get<R>(&mut self, in_buf: &mut R, loc: &[Loc]) -> Vec<u8>
    where
        R: Read + Seek,
    {

        let block_size = self.block_size as u32;

        // Calculate length from Loc
        // TODO: This underestimates, so we need to test it.
        let len = loc.iter().fold(0, |acc, x| acc + x.len(block_size));
        log::info!("BytesBlockStore Get: Calculated Length: {}", len);

        let mut result = Vec::with_capacity(len + 8192);

        if self.data.is_some() {
            let loc0 = loc[0].original_format(block_size);
            let loc1 = loc[loc.len() - 1].original_format(block_size);

            let start = loc0.0 as usize * block_size as usize + loc0.1 .0 as usize;
            let end = loc1.0 as usize * block_size as usize + loc1.1 .1 as usize;
            result.extend(&self.data.as_ref().unwrap()[start..=end]);
        } else {
            for (block, (start, end)) in loc.iter().map(|x| x.original_format(block_size)) {
                let block = self.get_block(in_buf, block);
                result.extend(&block[start as usize..=end as usize]);
            }
        }

        log::info!("BytesBlockStore Get: Result Length: {}", result.len());

        result
    }

    pub fn get_loaded(&self, loc: &[Loc]) -> Vec<u8> {
        let mut result = Vec::with_capacity(64);

        let block_size = self.block_size as u32;

        if self.data.is_some() {
            let loc0 = loc[0].original_format(block_size);
            let loc1 = loc[loc.len() - 1].original_format(block_size);

            let start = loc0.0 as usize * block_size as usize + loc0.1 .0 as usize;
            let end = loc1.0 as usize * block_size as usize + loc1.1 .1 as usize;
            result.extend(&self.data.as_ref().unwrap()[start..=end]);
        } else {
            panic!("Data not loaded");
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitvec::prelude::*;
    use std::io::Cursor;

    // Testing bitvec stuff for masking
    // also: https://docs.rs/bitvec/latest/bitvec/slice/struct.BitValIter.html
    #[test]
    fn test_bv_stuff() {
        let a: u8 = 5;
        let bv: BitVec<u8, Lsb0> = BitVec::from_element(a);
        let _b = bv[0..8].load::<u8>();
    }

    #[test]
    fn test_add_id() {
        let mut store = BytesBlockStore {
            block_size: 10,
            ..Default::default()
        };

        let test_ids = vec![
            "Medtr5g026775.t1",
            "ARABIDOPSIS_SUPER_COOL_GENE",
            "ID WITH A SPACE EVEN THOUGH ITS INVALID",
            "same, but lowercase....",
        ];

        let mut locs = Vec::new();

        for id in test_ids.iter() {
            locs.push(store.add(id.as_bytes()));
        }

        let mut buffer = Cursor::new(Vec::new());
        store.write_to_buffer(&mut buffer);

        let mut store = BytesBlockStore::from_buffer(&mut buffer, 0).unwrap();

        for i in 0..test_ids.len() {
            let id = store.get(&mut buffer, &locs[i]);
            println!("Locs: {:#?}", locs[i]);
            println!("{} {}", std::str::from_utf8(&id).unwrap(), test_ids[i]);
            assert_eq!(id, test_ids[i].as_bytes());
        }
    }
}
