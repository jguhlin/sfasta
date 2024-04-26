// TODO: https://docs.rs/bytes/latest/bytes/

use std::{
    io::{BufRead, Read, Seek, SeekFrom, Write},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use crossbeam::utils::Backoff;

use super::Builder;
use crate::datatypes::Loc;
use libcompression::*;
use libfractaltree::{FractalTreeBuild, FractalTreeDisk};

// Implement some custom errors to return
#[derive(Debug)]
pub enum BlockStoreError
{
    Empty,
    Other(&'static str),
}

/// This underlies most storage. It is a block store that stores bytes
/// of any type and compresses them Typically not used directly, but
/// used by sequence_block_store and string_block_store
// TODO: Should be a compression version and already-compressed
// version that doesn't use Arc<AtomicU64> and compression workers.
pub struct BytesBlockStoreBuilder
{
    /// Locations of the blocks in the file
    pub block_locations: Vec<Arc<AtomicU64>>,

    /// Locations of the block index (Where the serialized
    /// block_locations are stored)
    pub block_locations_pos: u64,

    /// Maximum block size
    pub block_size: usize,

    /// Data, typically a temporary store
    pub data: Vec<u8>, // Used for writing and reading...

    /// Compression configuration
    pub compression_config: Arc<CompressionConfig>,

    /// Compression worker. Enables multithreading for compression.
    pub compression_worker: Option<Arc<CompressionWorker>>,

    /// Whether the block store is finalized
    pub finalized: bool,

    create_dict: bool,
    dict_data: Vec<Vec<u8>>,
    dict_size: u64,
    dict_samples: u64,
}

unsafe impl Send for BytesBlockStoreBuilder {}
unsafe impl Sync for BytesBlockStoreBuilder {}

// The generic here is Vec<u8>
impl Builder<Vec<u8>> for BytesBlockStoreBuilder
{
    fn add(&mut self, input: Vec<u8>) -> Result<Vec<Loc>, &str>
    {
        self.add(input)
    }

    fn finalize(&mut self)
    {
        self.finalize();
    }
}

impl Default for BytesBlockStoreBuilder
{
    fn default() -> Self
    {
        BytesBlockStoreBuilder {
            block_locations_pos: 0,
            block_locations: Vec::new(),
            block_size: 8 * 1024,
            data: Vec::new(),
            compression_config: Arc::new(CompressionConfig::default()),
            compression_worker: None,
            finalized: false,
            create_dict: false,
            dict_data: Vec::new(),
            dict_size: 64 * 1024,
            dict_samples: 128,
        }
    }
}

impl BytesBlockStoreBuilder
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

    /// Configuration. Set the block size
    pub fn with_block_size(mut self, block_size: usize) -> Self
    {
        self.block_size = block_size;
        self.data = Vec::with_capacity(self.block_size);
        self
    }

    /// Configuration. Set the compressino config.
    pub fn with_compression(mut self, compression: CompressionConfig) -> Self
    {
        self.compression_config = Arc::new(compression);
        self
    }

    /// Set the compression worker.
    pub fn with_compression_worker(
        mut self,
        compression_worker: Arc<CompressionWorker>,
    ) -> Self
    {
        self.compression_worker = Some(compression_worker);
        self
    }

    /// Compress the current block
    fn compress_block(&mut self)
    {
        debug_assert!(self.compression_worker.is_some());

        let mut data = Vec::with_capacity(self.block_size);
        std::mem::swap(&mut self.data, &mut data);

        if self.create_dict {
            self.dict_data.push(data.clone());
            // Sum all the data for the dictionary
            let sum: usize = self.dict_data.iter().map(|x| x.len()).sum();

            // Std dict size is 100kb, they recommend at least 100x samples to
            // train
            if sum >= (self.dict_samples * self.dict_size) as usize {
                // Create the dict with the data we have...
                self.create_dict();
            }
        } else {
            let worker = self.compression_worker.as_ref().unwrap();
            let loc =
                worker.compress(data, Arc::clone(&self.compression_config));
            self.block_locations.push(loc);
        }
    }

    fn compress_final_block(&mut self)
    {
        assert!(self.compression_worker.is_some());

        if self.create_dict {
            self.create_dict();
        }

        self.compress_block();

        // Final block, so can be empty vec
        let mut data = Vec::new();
        std::mem::swap(&mut self.data, &mut data);

        let worker = self.compression_worker.as_ref().unwrap();
        let loc = worker.compress(data, Arc::clone(&self.compression_config));
        self.block_locations.push(loc);
    }

    pub fn create_dict(&mut self)
    {
        let dict =
            zstd::dict::from_samples(&self.dict_data, self.dict_size as usize);
        match dict {
            Ok(v) => {
                log::info!("Dict Size: {}", v.len());
                let mut cc = (*self.compression_config).clone();
                cc.compression_dict = Some(Arc::new(v));
                self.compression_config = Arc::new(cc);
            }
            Err(e) => {
                log::error!("Error creating dictionary: {}", e);
            }
        };

        self.create_dict = false;

        // Compress the data we have
        for data in self.dict_data.iter() {
            let worker = self.compression_worker.as_ref().unwrap();
            let loc = worker
                .compress(data.clone(), Arc::clone(&self.compression_config));
            self.block_locations.push(loc);
        }
    }

    /// Get number of blocks
    pub fn block_len(&self) -> usize
    {
        self.block_locations.len()
    }

    pub fn block_size(&self) -> usize
    {
        self.block_size
    }

    /// Check that all block locations are not 0
    pub fn check_complete(&self)
    {
        // If we aren't using a compression worker, don't need to check that
        // anything is complete
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
                    all_nonzero = false;
                    break;
                }
            }

            if !all_nonzero {
                backoff.snooze();
            }
        }
    }

    /// Add a sequence of bytes to the block store
    /// Returns a vector of Loc's that point to the location of the
    /// bytes in the block store (can span multiple blocks)
    pub fn add(&mut self, input: Vec<u8>) -> Result<Vec<Loc>, &str>
    {
        if self.finalized {
            panic!("Cannot add to finalized block store.");
        }

        let mut current_block = self.block_locations.len();
        let mut locs = Vec::with_capacity(8);

        let mut written = 0;
        let mut start_position = self.data.len();

        while written < input.len() {
            // How many bytes can we write to the current block?
            let remaining = self.block_size - self.data.len();

            // If we can write the entire input, do so
            if input.len() - written <= remaining as usize {
                self.data.extend_from_slice(&input[written..]);
                written += input[written..].len();

                // len is how many bytes are copied from the slice
                let len = (self.data.len() - start_position) as u32;

                locs.push(Loc {
                    block: current_block as u32,
                    start: start_position as u32,
                    len,
                });
            } else {
                self.data.extend_from_slice(
                    &input[written..(written + remaining as usize)],
                );
                written += remaining as usize;
                // len is how many bytes are copied from the slice
                let len = (self.data.len() - start_position) as u32;

                locs.push(Loc {
                    block: current_block as u32,
                    start: start_position as u32,
                    len,
                });
            }

            debug_assert!(self.data.len() <= self.block_size);
            if self.data.len() == self.block_size {
                self.compress_block();
                current_block += 1;
                start_position = self.data.len();
            }
        }

        Ok(locs)

        // let mut start = data.len();
        // data.extend_from_slice(input);
        // let end = data.len().saturating_sub(1);
        //
        // let compressed_blocks_count = self.block_locations.len();
        //
        // let starting_block = (start / self.block_size) +
        // compressed_blocks_count; let ending_block = (end /
        // self.block_size) + compressed_blocks_count;
        //
        // let mut locs = Vec::with_capacity((input.len() /
        // self.block_size).saturating_add(1));
        //
        // Process at block boundaries...
        // for block in starting_block..=ending_block {
        // let block_start = start % self.block_size;
        // let block_end = if block == ending_block {
        // (end % self.block_size).saturating_add(1)
        // } else {
        // self.block_size
        // };
        // start = block_end;
        //
        // let len = match block_end.checked_sub(block_start) {
        // Some(v) => v,
        // None => return Err("Block end < block start"),
        // };
        //
        // locs.push(Loc {
        // block: block as u32,
        // start: block_start as u32,
        // len: len as u32,
        // });
        // }
        //
        // Ok(locs)
    }

    /// Write out the header for BytesBlockStore
    pub fn write_header<W>(&mut self, pos: u64, mut out_buf: &mut W)
    where
        W: Write + Seek,
    {
        let bincode_config =
            bincode::config::standard().with_variable_int_encoding();

        out_buf.seek(SeekFrom::Start(pos)).unwrap();

        // Write the compression configuration
        bincode::encode_into_std_write(
            self.compression_config.as_ref(),
            &mut out_buf,
            bincode_config,
        )
        .unwrap();

        // Write the location of the block locations
        bincode::encode_into_std_write(
            self.block_locations_pos,
            &mut out_buf,
            bincode_config,
        )
        .unwrap();

        // Write out the block size
        bincode::encode_into_std_write(
            self.block_size,
            &mut out_buf,
            bincode_config,
        )
        .unwrap();
    }

    /// Writes the locations of each block. This is used for finding
    /// the start of each block.
    ///
    /// Splits the block locations into chunks of 1024 to create an
    /// index of block locations. TODO
    pub fn write_block_locations<W>(
        &mut self,
        mut out_buf: W,
    ) -> Result<(), BlockStoreError>
    where
        W: Write + Seek,
    {
        if !self.finalized {
            self.finalize();
        }

        self.check_complete();

        // No blocks, no data, exit out of this function.
        if self.block_locations.is_empty() {
            return Err(BlockStoreError::Empty);
        }

        let mut block_locations_tree: FractalTreeBuild<u32, u64> =
            FractalTreeBuild::new(2048, 8192);

        let block_locations: Vec<u64> = self
            .block_locations
            .iter()
            .map(|x| x.load(Ordering::Relaxed))
            .collect();
        assert!(block_locations.len() < u32::MAX as usize);
        block_locations.iter().enumerate().for_each(|(i, x)| {
            block_locations_tree.insert(i as u32, *x);
        });

        let mut block_locations_tree: FractalTreeDisk<_, _> =
            block_locations_tree.into();
        block_locations_tree.set_compression(CompressionConfig {
            compression_type: CompressionType::ZSTD,
            compression_level: 1,
            compression_dict: None,
        });
        let block_locations_pos =
            block_locations_tree.write_to_buffer(&mut out_buf).unwrap();

        self.block_locations_pos = block_locations_pos;

        Ok(())
    }

    // Push the last block into the compression queue
    pub fn finalize(&mut self)
    {
        assert!(self.compression_worker.is_some());

        if self.data.len() == 0 {
            return;
        }

        self.compress_block();
        self.compress_final_block();

        self.check_complete();

        self.finalized = true;
    }
}

/// This struct is for reading (and once finalized, writing)
#[derive(Debug)]
pub struct BytesBlockStore
{
    /// Locations of the blocks in the file
    // todo: fractal tree for all blocks across all datatypes...
    pub block_locations: FractalTreeDisk<u32, u64>,

    /// Locations of the block index (Where the serialized
    /// block_locations is stored)
    pub block_locations_pos: u64,

    /// Maximum block size
    pub block_size: usize,

    pub compression_config: CompressionConfig,

    /// Data, typically a temporary store
    pub data: Option<Vec<u8>>, // Used for writing and reading...

    /// Cache of the last block to speed up accesses
    cache: Option<(u32, Vec<u8>)>,
}

impl BytesBlockStore
{
    pub fn get_block<R>(&mut self, in_buf: &mut R, block: u32) -> Vec<u8>
    where
        R: Read + Seek + Send + Sync,
    {
        if self.cache.is_some() && self.cache.as_ref().unwrap().0 == block {
            return self.cache.as_ref().unwrap().1.clone();
        } else {
            let mut cache = match self.cache.take() {
                Some(x) => x,
                None => (block, vec![0; self.block_size]),
            };
            cache.0 = block;

            self.get_block_uncached(in_buf, block, &mut cache.1);
            self.cache = Some(cache);
            return self.cache.as_ref().unwrap().1.clone();
        }
    }

    pub fn get_block_uncached<R>(
        &mut self,
        mut in_buf: &mut R,
        block: u32,
        buffer: &mut [u8],
    ) where
        R: Read + Seek + Send + Sync,
    {
        let bincode_config_fixed = bincode::config::standard()
            .with_fixed_int_encoding()
            .with_limit::<{ 256 * 1024 * 1024 }>(); // 256 MB is max limit TODO: Enforce elsewhere too

        let block_location =
            self.block_locations.search(in_buf, &block).unwrap();

        in_buf.seek(SeekFrom::Start(block_location)).unwrap();
        let compressed_block: Vec<u8> =
            bincode::decode_from_std_read(&mut in_buf, bincode_config_fixed)
                .unwrap();

        let mut decompressor = zstd::bulk::Decompressor::new().unwrap();
        decompressor.include_magicbytes(false).unwrap();
        decompressor
            .decompress_to_buffer(&compressed_block, buffer)
            .unwrap();
    }

    pub fn get<R>(&mut self, in_buf: &mut R, loc: &[Loc]) -> Vec<u8>
    where
        R: Read + Seek + Send + Sync,
    {
        let block_size = self.block_size as u32;

        // Calculate length from Loc
        // TODO: This underestimates, so we need to test it.
        // let len = loc.iter().fold(0, |acc, x| acc + x.len as usize);

        // let mut result = Vec::with_capacity(len + 8192);
        let mut result = Vec::new();

        if self.data.is_some() {
            let loc0 = &loc[0];
            let loc1 = &loc[loc.len() - 1];

            let start =
                loc0.block as usize * block_size as usize + loc0.start as usize;
            let end = loc1.block as usize * block_size as usize
                + loc1.start as usize
                + loc1.len as usize;

            // #[cfg(test)]
            // assert!(start + len <= block_size as usize);

            result.extend_from_slice(&self.data.as_ref().unwrap()[start..end]);
        } else {
            for l in loc.iter().map(|x| x) {
                let block = self.get_block(in_buf, l.block);

                #[cfg(test)]
                assert!(l.start + l.len <= block_size);

                let end = l.start as usize + l.len as usize;

                result.extend_from_slice(&block[l.start as usize..end]);
            }
        }

        result
    }

    pub fn get_loaded(&self, loc: &[Loc]) -> Vec<u8>
    {
        let mut result = Vec::with_capacity(64);

        let block_size = self.block_size as u32;

        if self.data.is_some() {
            let loc0 = &loc[0];
            let loc1 = &loc[loc.len() - 1];

            let start =
                loc0.block as usize * block_size as usize + loc0.start as usize;
            let end = loc1.block as usize * block_size as usize
                + loc1.start as usize
                + loc1.len as usize;
            result.extend(&self.data.as_ref().unwrap()[start..=end]);
        } else {
            panic!("Data not loaded");
        }

        result
    }

    /// Read header from a buffer into a BytesBlockStore
    pub fn from_buffer<R>(
        mut in_buf: &mut R,
        starting_pos: u64,
    ) -> Result<Self, String>
    where
        R: Read + Seek + Send + Sync + BufRead,
    {
        let bincode_config = bincode::config::standard()
            .with_variable_int_encoding()
            .with_limit::<8192>();

        in_buf.seek(SeekFrom::Start(starting_pos)).unwrap();
        let (compression_config, block_locations_pos, block_size) =
            match bincode::decode_from_std_read(&mut in_buf, bincode_config) {
                Ok(x) => x,
                Err(e) => {
                    return Err(format!("Error decoding block store: {e}"))
                }
            };

        assert!(block_locations_pos > 0);

        let block_locations: FractalTreeDisk<u32, u64> =
            FractalTreeDisk::from_buffer(&mut in_buf, block_locations_pos)
                .unwrap();

        Ok(BytesBlockStore {
            block_locations,
            block_locations_pos,
            block_size,
            data: None,
            cache: None,
            compression_config,
        })
    }

    pub fn block_size(&self) -> usize
    {
        self.block_size
    }
}

// TODO: More tests
#[cfg(test)]
mod tests
{
    use super::*;

    #[test]
    fn test_add_id()
    {
        let output_buffer = Arc::new(std::sync::Mutex::new(Box::new(
            std::io::Cursor::new(Vec::with_capacity(1024 * 1024)),
        )));

        let mut output_worker = crate::io::worker::Worker::new(output_buffer)
            .with_buffer_size(1024);
        output_worker.start();

        let output_queue = output_worker.get_queue();

        let mut compression_workers = CompressionWorker::new()
            .with_buffer_size(16)
            .with_threads(1_u16)
            .with_output_queue(Arc::clone(&output_queue));

        compression_workers.start();
        let compression_workers = Arc::new(compression_workers);

        let mut store = BytesBlockStoreBuilder {
            block_size: 10,
            compression_config: Arc::new(CompressionConfig {
                compression_type: CompressionType::NONE,
                compression_level: 0,
                compression_dict: None,
            }),
            compression_worker: Some(Arc::clone(&compression_workers)),
            ..Default::default()
        };

        let loc = store.add(b"Medtr5g026775.t1".to_vec()).unwrap();
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

        let mut store = BytesBlockStoreBuilder {
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
            locs.push(store.add(id.as_bytes().to_vec()).unwrap());
        }
    }
}
