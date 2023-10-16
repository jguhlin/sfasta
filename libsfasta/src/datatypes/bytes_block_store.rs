use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::atomic::Ordering;
use std::sync::{atomic::AtomicU64, Arc};

use binout::{Serializer, VByte};
use crossbeam::utils::Backoff;

use crate::compression::{CompressionConfig, CompressionType, Worker};
use crate::datatypes::Loc;

pub struct BytesBlockStore {
    location: u64,
    block_locations: Vec<Arc<AtomicU64>>,
    block_locations_pos: Arc<AtomicU64>,
    block_size: usize,
    pub data: Option<Vec<u8>>, // Used for writing and reading...
    pub compression_config: Arc<CompressionConfig>,
    cache: Option<(u32, Vec<u8>)>,
    compression_worker: Option<Arc<Worker>>,
    index_block_size: usize,
    finalized: bool,
}

impl Default for BytesBlockStore {
    fn default() -> Self {
        BytesBlockStore {
            location: 0,
            block_locations_pos: Arc::new(AtomicU64::new(0)),
            block_locations: Vec::new(),
            block_size: 512 * 1024,
            data: None,
            cache: None,
            compression_config: Arc::new(CompressionConfig::default()),
            compression_worker: None,
            index_block_size: 1024,
            finalized: false,
        }
    }

    /*
    #[cfg(test)]
    fn default() -> Self {

        let output_buffer = Arc::new(std::sync::Mutex::new(Box::new(std::io::Cursor::new(
            Vec::with_capacity(1024 * 1024),
        ))));

        let mut output_worker = crate::io::worker::Worker::new(output_buffer).with_buffer_size(16);
        output_worker.start();
        let output_queue = output_worker.get_queue();

        let mut compression_workers = crate::compression::worker::Worker::new()
            .with_buffer_size(16)
            .with_threads(1_u16)
            .with_output_queue(Arc::clone(&output_queue));

        compression_workers.start();
        let compression_workers = Arc::new(compression_workers);

        BytesBlockStore {
            location: 0,
            block_locations_index: None, // TODO
            block_locations_pos: Arc::new(AtomicU64::new(0)),
            block_locations: None,
            block_size: 512 * 1024,
            data: None,
            cache: None,
            compression_config: Arc::new(CompressionConfig::default()),
            compression_worker: Some(Arc::clone(&compression_workers)),
            index_block_size: 1024,
        }
    } */
}

impl BytesBlockStore {
    pub fn block_len(&self) -> usize {
        self.block_locations.len()
    }

    pub fn with_block_size(mut self, block_size: usize) -> Self {
        self.block_size = block_size;
        self
    }

    pub fn with_compression(mut self, compression: CompressionConfig) -> Self {
        self.compression_config = Arc::new(compression);
        self
    }

    pub fn with_compression_worker(mut self, compression_worker: Arc<Worker>) -> Self {
        self.compression_worker = Some(compression_worker);
        self
    }

    fn compress_block(&mut self) {
        assert!(self.compression_worker.is_some());
        assert!(self.data.is_some());

        let at = std::cmp::min(self.block_size, self.data.as_mut().unwrap().len());

        let mut block = self.data.as_mut().unwrap().split_off(at);
        std::mem::swap(&mut block, self.data.as_mut().unwrap());

        let worker = self.compression_worker.as_ref().unwrap();
        let loc = worker.compress(block, Arc::clone(&self.compression_config));
        self.block_locations.push(loc);
    }

    pub fn check_complete(&self) {
        // If nowhere to go, then don't bother here...
        if self.compression_worker.is_none() {
            return;
        }

        // Check that all block_locations are not 0, or backoff if any are
        let block_locations = &self.block_locations;
        let backoff = Backoff::new();
        let mut all_nonzero = false;

        while !all_nonzero {
            all_nonzero = true;
            for loc in block_locations.iter() {
                if loc.load(Ordering::Relaxed) == 0 {
                    // println!("Loc: {}", loc.load(Ordering::Relaxed));
                    all_nonzero = false;
                    break;
                }
            }

            if !all_nonzero {
                backoff.snooze();
            }
        }
    }

    pub fn add<'b>(&'b mut self, input: &[u8]) -> Result<Vec<Loc>, &str> {
        if self.finalized {
            panic!("Cannot add to finalized block store");
        }

        if self.data.is_none() {
            self.data = Some(Vec::with_capacity(self.block_size));
        }

        if self.data.as_ref().unwrap().len() > self.block_size {
            self.compress_block();
        }

        let data = self.data.as_mut().unwrap();

        let mut start = data.len();
        data.extend(input);
        let end = data.len() - 1;

        let compressed_blocks_count = self.block_locations.len();

        let starting_block = (start / self.block_size) + compressed_blocks_count;
        let ending_block = (end / self.block_size) + compressed_blocks_count;

        let mut locs = Vec::with_capacity(input.len() / self.block_size + 1);

        // Process at block boundaries...
        for block in starting_block..=ending_block {
            let block_start = start % self.block_size;
            let block_end = if block == ending_block {
                (end % self.block_size) + 1
            } else {
                self.block_size
            };
            start = block_end;

            let len = match block_end.checked_sub(block_start) {
                Some(v) => v,
                None => return Err("Block end < block start"),
            };

            locs.push(Loc {
                block: block as u32,
                start: block_start as u32,
                len: len as u32,
            });
        }

        Ok(locs)
    }

    /// Write out the header for BytesBlockStore
    pub fn write_header<W>(&mut self, pos: u64, mut out_buf: &mut W)
    where
        W: Write + Seek,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        out_buf.seek(SeekFrom::Start(pos)).unwrap();

        bincode::encode_into_std_write(&self.compression_config, &mut out_buf, bincode_config)
            .unwrap();
        bincode::encode_into_std_write(
            &self.block_locations_pos.load(Ordering::Relaxed),
            &mut out_buf,
            bincode_config,
        )
        .unwrap();
        bincode::encode_into_std_write(self.block_size, &mut out_buf, bincode_config).unwrap();
    }

    /// Writes the locations of each block. This is used for finding the start of each block.
    ///
    /// Splits the block locations into chunks of 1024 to create an index of block locations. TODO
    pub fn write_block_locations(&mut self) {
        if !self.finalized {
            self.finalize();
        }

        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        assert!(self.compression_worker.is_some());
        self.check_complete();

        if self.block_locations.is_empty() {
            return;
        }

        let mut output: Vec<u8> = Vec::with_capacity(self.index_block_size * 10 + 256);

        let block_locations: Vec<u64> = self
            .block_locations
            .iter()
            .map(|x| x.load(Ordering::Relaxed))
            .collect();

        VByte::write_array(&mut output, &block_locations[..]).unwrap();

        let bincoded = bincode::encode_to_vec(&output, bincode_config).unwrap();

        let compression_config = Arc::new(CompressionConfig {
            compression_type: CompressionType::NONE,
            compression_level: 0,
            compression_dict: None,
        });

        let worker = self.compression_worker.as_ref().unwrap();
        let loc = worker.compress(bincoded, compression_config);
        self.block_locations_pos = loc;

        while self.block_locations_pos.load(Ordering::Relaxed) == 0 {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    pub fn from_buffer<R>(mut in_buf: &mut R, starting_pos: u64) -> Result<Self, String>
    where
        R: Read + Seek,
    {
        let bincode_config = bincode::config::standard()
            .with_fixed_int_encoding()
            .with_limit::<1024>();

        let mut store = BytesBlockStore::default();

        in_buf.seek(SeekFrom::Start(starting_pos)).unwrap();
        (
            store.compression_config,
            store.block_locations_pos,
            store.block_size,
        ) = match bincode::decode_from_std_read(&mut in_buf, bincode_config) {
            Ok(x) => x,
            Err(e) => return Err(format!("Error decoding block store: {e}")),
        };

        store.location = starting_pos;

        in_buf
            .seek(SeekFrom::Start(
                store.block_locations_pos.load(Ordering::Relaxed),
            ))
            .unwrap();

        if store.block_locations_pos.load(Ordering::Relaxed) > 0 {
            let bincode_config = bincode::config::standard().with_fixed_int_encoding();

            let compressed: Vec<u8> =
                match bincode::decode_from_std_read(&mut in_buf, bincode_config) {
                    Ok(x) => x,
                    Err(e) => return Err(format!("Error decoding block locations: {e}")),
                };

            // let block_locations: Vec<u8> = zstd::stream::decode_all(&compressed[..]).unwrap();
            // let block_locations: Vec<u64> =
            // match bincode::decode_from_slice(&block_locations, bincode_config) {
            //   Ok(x) => x.0,
            //Err(e) => return Err(format!("Error decoding block locations: {e}")),
            //};
            let block_locations: Vec<u64> =
                VByte::read_array(&mut &compressed[..]).unwrap().to_vec();

            // Convert to Arc<AtomicU64>'s...
            // TODO: Do something else since this probably adds a bit of time...
            let block_locations = block_locations
                .into_iter()
                .map(|x| Arc::new(AtomicU64::new(x)))
                .collect();

            store.block_locations = block_locations;
        }

        Ok(store)
    }

    pub fn prefetch<R>(&mut self, in_buf: &mut R)
    where
        R: Read + Seek,
    {
        let mut data = Vec::with_capacity(self.block_size * self.block_locations.len());

        for i in 0..self.block_locations.len() {
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
            .with_limit::<{ 256 * 1024 * 1024 }>(); // 256 MB is max limit TODO: Enforce elsewhere too

        let block_locations = &self.block_locations;

        let mut decompressor = zstd::bulk::Decompressor::new().unwrap();
        decompressor.include_magicbytes(false).unwrap();

        let block_location = &block_locations[block as usize];
        let block_location = block_location.load(Ordering::Relaxed) as u64;
        in_buf.seek(SeekFrom::Start(block_location)).unwrap();
        let compressed_block: Vec<u8> =
            bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();

        let out = decompressor
            .decompress(&compressed_block, self.block_size)
            .unwrap();

        out
    }

    pub fn get<R>(&mut self, in_buf: &mut R, loc: &[Loc]) -> Vec<u8>
    where
        R: Read + Seek,
    {
        let block_size = self.block_size as u32;

        // Calculate length from Loc
        // TODO: This underestimates, so we need to test it.
        let len = loc.iter().fold(0, |acc, x| acc + x.len as usize);
        log::debug!("BytesBlockStore Get: Calculated Length: {}", len);

        let mut result = Vec::with_capacity(len + 8192);

        if self.data.is_some() {
            let loc0 = &loc[0];
            let loc1 = &loc[loc.len() - 1];

            let start = loc0.block as usize * block_size as usize + loc0.start as usize;
            let end =
                loc1.block as usize * block_size as usize + loc1.start as usize + loc1.len as usize;

            #[cfg(test)]
            assert!(start + len <= block_size as usize);

            result.extend(&self.data.as_ref().unwrap()[start..end]);
        } else {
            for l in loc.iter().map(|x| x) {
                let block = self.get_block(in_buf, l.block);

                #[cfg(test)]
                assert!(l.start + l.len <= block_size);

                let end = l.start as usize + l.len as usize;

                result.extend(&block[l.start as usize..end]);
            }
        }

        log::debug!("BytesBlockStore Get: Result Length: {}", result.len());

        result
    }

    pub fn get_loaded(&self, loc: &[Loc]) -> Vec<u8> {
        let mut result = Vec::with_capacity(64);

        let block_size = self.block_size as u32;

        if self.data.is_some() {
            let loc0 = &loc[0];
            let loc1 = &loc[loc.len() - 1];

            let start = loc0.block as usize * block_size as usize + loc0.start as usize;
            let end =
                loc1.block as usize * block_size as usize + loc1.start as usize + loc1.len as usize;
            result.extend(&self.data.as_ref().unwrap()[start..=end]);
        } else {
            panic!("Data not loaded");
        }

        result
    }

    // Push the last block into the compression queue
    pub fn finalize(&mut self) {
        assert!(self.compression_worker.is_some());

        if self.data.is_none() {
            return;
        }

        let mut block: Option<Vec<u8>> = None;
        std::mem::swap(&mut self.data, &mut block);

        if block.is_some() && block.as_ref().unwrap().len() > 0 {
            let worker = self.compression_worker.as_ref().unwrap();
            let loc = worker.compress(block.unwrap(), Arc::clone(&self.compression_config));
            self.block_locations.push(loc);
        }

        self.check_complete();

        self.finalized = true;
    }
}

// TODO: More tests
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_boundaries() {
        let repeated_data: Vec<u8> = vec![0; 1000];

        let mut store = BytesBlockStore {
            block_size: 64,
            ..Default::default()
        };

        let locs = store.add(&repeated_data).unwrap();
        println!("{:?}", locs);
    }

    #[test]
    fn test_add_id() {
        let output_buffer = Arc::new(std::sync::Mutex::new(Box::new(std::io::Cursor::new(
            Vec::with_capacity(1024 * 1024),
        ))));

        let mut output_worker =
            crate::io::worker::Worker::new(output_buffer).with_buffer_size(1024);
        output_worker.start();

        let output_queue = output_worker.get_queue();

        let mut compression_workers = crate::compression::worker::Worker::new()
            .with_buffer_size(16)
            .with_threads(1_u16)
            .with_output_queue(Arc::clone(&output_queue));

        compression_workers.start();
        let compression_workers = Arc::new(compression_workers);

        let mut store = BytesBlockStore {
            block_size: 10,
            compression_config: Arc::new(CompressionConfig {
                compression_type: CompressionType::NONE,
                compression_level: 0,
                compression_dict: None,
            }),
            compression_worker: Some(Arc::clone(&compression_workers)),
            ..Default::default()
        };

        let loc = store.add(b"Medtr5g026775.t1").unwrap();
        println!("{:?}", loc);
        assert!(
            loc == vec![
                Loc {
                    block: 0,
                    start: 0,
                    len: 10
                },
                Loc {
                    block: 1,
                    start: 0,
                    len: 6
                }
            ]
        );

        let mut store = BytesBlockStore {
            block_size: 10,
            compression_worker: Some(Arc::clone(&compression_workers)),
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
            locs.push(store.add(id.as_bytes()).unwrap());
        }
    }
}
