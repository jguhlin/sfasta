use std::{
    io::{BufRead, Read, Seek, SeekFrom, Write},
    pin::{self, Pin},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use stream_vbyte::{
    encode::encode,
    decode::{decode, cursor::DecodeCursor},
    scalar::Scalar
};

use hashbrown::HashMap;

#[cfg(feature = "async")]
use async_stream::stream;

#[cfg(feature = "async")]
use tokio_stream::Stream;

#[cfg(feature = "async")]
use libfilehandlemanager::AsyncFileHandleManager;

#[cfg(feature = "async")]
use tokio::{
    fs::File,
    io::{AsyncSeekExt, BufReader},
    sync::{
        mpsc, mpsc::Receiver, mpsc::Sender, oneshot, Mutex,
        OwnedRwLockWriteGuard, RwLock,
    },
};

use bytes::{Buf, BufMut, Bytes, BytesMut};

use crossbeam::utils::Backoff;

use super::Builder;
use crate::datatypes::Loc;
use libcompression::*;
use libfractaltree::{FractalTreeBuild, FractalTreeDisk};

#[cfg(feature = "async")]
use libfractaltree::FractalTreeDiskAsync;

#[cfg(feature = "async")]
use crate::parser::async_parser::{
    bincode_decode_from_buffer_async,
    bincode_decode_from_buffer_async_with_size_hint,
    bincode_decode_from_buffer_async_with_size_hint_nc,
};

#[cfg(feature = "async")]
use tokio_stream::StreamExt;

// Implement some custom errors to return
#[derive(Debug)]
pub enum BlockStoreError
{
    Empty,
    Other(&'static str),
}

#[cfg(feature = "async")]
pub enum DataOrLater
{
    Data(Bytes),
    Later(oneshot::Receiver<Bytes>),
}

#[cfg(feature = "async")]
impl DataOrLater
{
    pub async fn await_data(self) -> Bytes
    {
        match self {
            DataOrLater::Data(data) => data,
            DataOrLater::Later(receiver) => match receiver.await {
                Ok(data) => data,
                Err(e) => panic!("Error receiving data: {}", e),
            },
        }
    }
}
/// This underlies most storage. It is a block store that stores bytes
/// of any type and compresses them Typically not used directly, but
/// used by sequence_block_store and string_block_store
// TODO: Should be a compression version and already-compressed
// version that doesn't use Arc<AtomicU64> and compression workers.
pub struct BytesBlockStoreBuilder
{
    /// Locations of the blocks in the file
    pub block_data: Vec<(Arc<AtomicU64>, Arc<AtomicU64>, usize)>,

    /// Locations of the block index (Where the serialized
    /// block_locations are stored)
    pub block_locations_pos: u64,

    /// Maximum block size
    pub block_size: u32,

    /// Data, typically a temporary store
    pub data: Vec<u8>, // Used for writing and reading...

    /// Tree Compression configuration
    pub tree_compression_config: CompressionConfig,

    /// Compression config for the block store
    pub compression_config: Arc<CompressionConfig>,

    /// Compression worker. Enables multithreading for compression.
    pub compression_worker: Option<Arc<CompressionWorker>>,

    /// Whether the block store is finalized
    pub finalized: bool,

    create_dict: bool,
    dict_data: Vec<Vec<u8>>,
    dict_size: u64,
    dict_samples: u64,

    // Statistics
    pub data_size: usize,
    pub compressed_size: usize,
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

    fn finalize(&mut self) -> Result<(), &str>
    {
        match self.finalize() {
            Ok(_) => Ok(()),
            Err(_) => Err("Unable to finalize block store"),
        }
    }
}

impl Default for BytesBlockStoreBuilder
{
    fn default() -> Self
    {
        BytesBlockStoreBuilder {
            block_locations_pos: 0,
            block_data: Vec::new(),
            block_size: 64 * 1024,
            data: Vec::new(),
            tree_compression_config: CompressionConfig::default(),
            compression_config: Arc::new(CompressionConfig::default()),
            compression_worker: None,
            finalized: false,
            create_dict: false,
            dict_data: Vec::new(),
            dict_size: 64 * 1024,
            dict_samples: 128,
            data_size: 0,
            compressed_size: 0,
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
        self.block_size = block_size as u32;
        self.data = Vec::with_capacity(self.block_size as usize);
        self
    }

    /// Configuration. Set the compression config.
    pub fn with_tree_compression(
        mut self,
        compression: CompressionConfig,
    ) -> Self
    {
        self.tree_compression_config = compression;
        self
    }

    /// Configuration. Set the compression config.
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

        let mut data = Vec::with_capacity(self.block_size as usize);
        std::mem::swap(&mut self.data, &mut data);

        self.data_size += data.len();
        let data_len = data.len();

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
            self.block_data.push((loc.0, loc.1, data_len));
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

        let data_len = data.len();

        let worker = self.compression_worker.as_ref().unwrap();
        let loc = worker.compress(data, Arc::clone(&self.compression_config));
        self.block_data.push((loc.0, loc.1, data_len));
    }

    pub fn create_dict(&mut self)
    {
        let dict =
            zstd::dict::from_samples(&self.dict_data, self.dict_size as usize);
        match dict {
            Ok(v) => {
                log::info!("Dict Size: {}", v.len());
                let mut cc = (*self.compression_config).clone();
                cc.compression_dict = Some(v);
                self.compression_config = Arc::new(cc);
            }
            Err(e) => {
                log::error!("Error creating dictionary: {}", e);
            }
        };

        self.create_dict = false;

        // Compress the data we have
        for data in self.dict_data.iter() {
            let data_len = data.len();
            let worker = self.compression_worker.as_ref().unwrap();
            let loc = worker
                .compress(data.clone(), Arc::clone(&self.compression_config));
            self.block_data.push((loc.0, loc.1, data_len));
        }
    }

    /// Get number of blocks
    pub fn block_len(&self) -> usize
    {
        self.block_data.len()
    }

    pub fn block_size(&self) -> usize
    {
        self.block_size as usize
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
        let block_locations =
            &self.block_data.iter().map(|x| &x.0).collect::<Vec<_>>();
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

        let mut current_block = self.block_data.len();
        let mut locs = Vec::with_capacity(8);

        let mut written = 0;
        let mut start_position = self.data.len();

        while written < input.len() {
            // How many bytes can we write to the current block?
            let remaining = self.block_size as usize - self.data.len();

            // If we can write the entire input, do so
            if input.len() - written <= remaining as usize {
                // todo the following is repeated twice, make a fn
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
                // If the input could fit into a new block, and this one is
                // mostly full, let's just finish this block and start a new one
                // (to avoid opening two blocks for small seq)
                // log::debug!("Taking shortcut block: BS: {} Input: {} W: {} R:
                // {} R05: {}", self.block_size, input.len(), written,
                // remaining, 0.05 * self.block_size as f32);
                if remaining < (0.05 * self.block_size as f32) as usize
                    && written == 0
                {
                    self.compress_block();
                    current_block += 1;
                    start_position = self.data.len();
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
            }

            debug_assert!(self.data.len() <= self.block_size as usize);
            if self.data.len() == self.block_size as usize {
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
            self.finalize().expect("Error finalizing block store");
        }

        self.check_complete();

        // No blocks, no data, exit out of this function.
        if self.block_data.is_empty() {
            return Err(BlockStoreError::Empty);
        }

        let mut block_locations_tree: FractalTreeBuild<u32, u64> =
            FractalTreeBuild::new(512, 1024);

        self.compressed_size = self
            .block_data
            .iter()
            .map(|x| x.1.load(Ordering::Relaxed) as usize)
            .sum::<usize>();

        let block_locations: Vec<u64> = self
            .block_data
            .iter()
            .map(|x| x.0.load(Ordering::Relaxed))
            .collect();
        assert!(block_locations.len() < u32::MAX as usize);
        block_locations.iter().enumerate().for_each(|(i, x)| {
            block_locations_tree.insert(i as u32, *x);
        });

        let mut block_locations_tree: FractalTreeDisk<_, _> =
            block_locations_tree.into();
        block_locations_tree
            .set_compression(self.tree_compression_config.clone());
        let block_locations_pos =
            block_locations_tree.write_to_buffer(&mut out_buf).unwrap();

        self.block_locations_pos = block_locations_pos;

        Ok(())
    }

    // Push the last block into the compression queue
    pub fn finalize(&mut self) -> Result<(), BlockStoreError>
    {
        assert!(self.compression_worker.is_some());

        if self.data.len() == 0 {
            return Ok(());
        }

        self.compress_block();
        self.compress_final_block();

        self.check_complete();

        self.finalized = true;
        Ok(())
    }
}

/// This struct is for reading (and once finalized, writing)
pub struct BytesBlockStore
{
    /// Locations of the blocks in the file
    // todo: fractal tree for all blocks across all datatypes...
    #[cfg(not(feature = "async"))]
    pub block_locations: FractalTreeDisk<u32, u64>,

    #[cfg(feature = "async")]
    pub block_locations: Arc<FractalTreeDiskAsync<u32, u64>>,

    /// Locations of the block index (Where the serialized
    /// block_locations is stored)
    pub block_locations_pos: u64,

    /// Maximum block size
    pub block_size: u32,

    pub compression_config: CompressionConfig,

    /// Data, typically a temporary store
    pub data: Option<Vec<u8>>, // Used for writing and reading...

    /// Cache of the last block to speed up accesses
    #[cfg(not(feature = "async"))]
    cache: Option<(u32, Vec<u8>)>,

    #[cfg(feature = "async")]
    cache: Arc<AsyncLRU>,

    #[cfg(feature = "async")]
    block_jobs: Arc<Mutex<Vec<(u32, Vec<oneshot::Sender<Bytes>>)>>>,
}

impl BytesBlockStore
{
    #[cfg(not(feature = "async"))]
    pub fn get_block<R>(&mut self, in_buf: &mut R, block: u32) -> Vec<u8>
    where
        R: Read + Seek + Send + Sync,
    {
        if self.cache.is_some() && self.cache.as_ref().unwrap().0 == block {
            return self.cache.as_ref().unwrap().1.clone();
        } else {
            let mut cache = match self.cache.take() {
                Some(x) => x,
                None => (block, vec![0; self.block_size as usize]),
            };
            cache.0 = block;

            self.get_block_uncached(in_buf, block, &mut cache.1);
            self.cache = Some(cache);
            return self.cache.as_ref().unwrap().1.clone();
        }
    }

    #[cfg(feature = "async")]
    #[tracing::instrument(skip(self, in_buf))]
    pub async fn get_block(
        &self,
        in_buf: &mut tokio::sync::OwnedMutexGuard<BufReader<File>>,
        block: u32,
    ) -> DataOrLater
    {
        // If it's cached, return immediately...
        if let Some(stored) = self.cache.get(block).await {
            return DataOrLater::Data(stored);
        }

        // If not cached, see if anyone else is fetching this block
        let mut block_jobs = self.block_jobs.lock().await;
        for (b, senders) in block_jobs.iter_mut() {
            if *b == block {
                let (sender, receiver) = oneshot::channel();
                senders.push(sender);
                return DataOrLater::Later(receiver);
            }
        }

        // If no one is fetching this block, start fetching it
        block_jobs.push((block, Vec::new()));

        // Drop the lock
        drop(block_jobs);

        let bincode_config_fixed = bincode::config::standard()
            .with_fixed_int_encoding()
            .with_limit::<{ 256 * 1024 * 1024 }>(); // 256 MB is max limit TODO: Enforce elsewhere too

        let block_location = self.block_locations.search(&block).await.unwrap();

        in_buf.seek(SeekFrom::Start(block_location)).await.unwrap();
        let compressed_block: Vec<u8> =
            bincode_decode_from_buffer_async_with_size_hint_nc(
                in_buf,
                bincode_config_fixed,
                self.block_size as usize,
            )
            .await
            .unwrap();

        // todo unnecessary allocations, also should be async
        // Copy this Vec<u8> into the buffer
        // Lengths may be different
        let compression = self.compression_config.clone();
        let decompressed = tokio::task::spawn_blocking(move || {
            Bytes::from(compression.decompress(&compressed_block).unwrap())
        });

        let decompressed = decompressed.await.unwrap();

        // Grab the block_jobs again
        let mut block_jobs = self.block_jobs.lock().await;

        // Send the block to all the waiting tasks
        for (b, senders) in block_jobs.iter_mut() {
            if *b == block {
                for sender in senders.drain(..) {
                    sender.send(decompressed.clone()).unwrap();
                }
                break;
            }
        }

        block_jobs.retain(|x| x.0 != block);
        drop(block_jobs);

        DataOrLater::Data(self.cache.add(block, decompressed.clone()).await)
        // DataOrLater::Data(decompressed)
    }

    /// Bypasses the block_locations lookup
    #[cfg(feature = "async")]
    #[tracing::instrument(skip(self, in_buf))]
    pub async fn get_block_pos_known(
        &self,
        in_buf: &mut tokio::sync::OwnedMutexGuard<BufReader<File>>,
        block: u32,
        block_location: u64,
    ) -> DataOrLater
    {
        // If it's cached, return immediately...
        if let Some(stored) = self.cache.get(block).await {
            return DataOrLater::Data(stored);
        }

        // If not cached, see if anyone else is fetching this block
        let mut block_jobs = self.block_jobs.lock().await;
        for (b, senders) in block_jobs.iter_mut() {
            if *b == block {
                let (sender, receiver) = oneshot::channel();
                senders.push(sender);
                return DataOrLater::Later(receiver);
            }
        }

        // If no one is fetching this block, start fetching it
        block_jobs.push((block, Vec::new()));

        // Drop the lock
        drop(block_jobs);

        let bincode_config_fixed = bincode::config::standard()
            .with_fixed_int_encoding()
            .with_limit::<{ 256 * 1024 * 1024 }>(); // 256 MB is max limit TODO: Enforce elsewhere too

        in_buf.seek(SeekFrom::Start(block_location)).await.unwrap();
        let compressed_block: Vec<u8> =
            bincode_decode_from_buffer_async_with_size_hint_nc(
                in_buf,
                bincode_config_fixed,
                self.block_size as usize,
            )
            .await
            .unwrap();

        // todo unnecessary allocations, also should be async
        // Copy this Vec<u8> into the buffer
        // Lengths may be different
        let compression = self.compression_config.clone();
        // let decompressed = tokio::task::spawn_blocking(move || {
        // Bytes::from(compression.decompress(&compressed_block).unwrap())
        // });

        let decompressed =
            Bytes::from(compression.decompress(&compressed_block).unwrap());

        // let decompressed = decompressed.await.unwrap();

        // Grab the block_jobs again
        let mut block_jobs = self.block_jobs.lock().await;

        // Send the block to all the waiting tasks
        for (b, senders) in block_jobs.iter_mut() {
            if *b == block {
                for sender in senders.drain(..) {
                    sender.send(decompressed.clone()).unwrap();
                }
                break;
            }
        }

        block_jobs.retain(|x| x.0 != block);
        drop(block_jobs);

        DataOrLater::Data(self.cache.add(block, decompressed.clone()).await)
    }

    #[cfg(not(feature = "async"))]
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

        // todo unnecessary allocations, also should be async
        // Copy this Vec<u8> into the buffer
        // Lengths may be different
        let decompressed = self
            .compression_config
            .decompress(&compressed_block)
            .unwrap();

        buffer[..decompressed.len()].copy_from_slice(&decompressed);
    }

    #[cfg(not(feature = "async"))]
    pub fn get<R>(&mut self, in_buf: &mut R, loc: &[Loc]) -> bytes::Bytes
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

            debug_assert!(end > start);
            debug_assert!(start + loc1.len as usize <= block_size as usize);
            debug_assert!(start + end <= block_size as usize);

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

        Bytes::from(result)
    }

    #[cfg(feature = "async")]
    #[tracing::instrument(skip(self, in_buf))]
    pub async fn get(
        &self,
        in_buf: &mut tokio::sync::OwnedMutexGuard<BufReader<File>>,
        loc: &[Loc],
    ) -> Bytes
    {
        let block_size = self.block_size as u32;

        // Calculate length from Loc
        let mut result = Vec::with_capacity(loc.len());
        for l in loc.iter() {
            let block = match self.get_block(in_buf, l.block).await {
                DataOrLater::Data(data) => data,
                DataOrLater::Later(data) => data.await.unwrap(),
            };

            debug_assert!(l.start + l.len <= block_size);

            let end = l.start as usize + l.len as usize;

            result.extend_from_slice(&block[l.start as usize..end]);
        }
        Bytes::from(result)
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

    #[cfg(not(feature = "async"))]
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

        // assert!(block_locations_pos > 0);

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

    #[cfg(feature = "async")]
    /// Read header from a buffer into a BytesBlockStore
    #[tracing::instrument(skip(in_buf))]
    pub async fn from_buffer(
        in_buf: &mut BufReader<File>,
        filename: String,
        starting_pos: u64,
    ) -> Result<Self, String>
    {
        let bincode_config = bincode::config::standard()
            .with_variable_int_encoding()
            .with_limit::<8192>();

        in_buf.seek(SeekFrom::Start(starting_pos)).await.unwrap();
        let (compression_config, block_locations_pos, block_size) =
            match bincode_decode_from_buffer_async_with_size_hint::<
                { 64 * 1024 },
                _,
                _,
            >(in_buf, bincode_config)
            .await
            {
                Ok(x) => x,
                Err(e) => {
                    return Err(format!("Error decoding block store: {e}"))
                }
            };

        // assert!(block_locations_pos > 0);

        let block_locations: Arc<FractalTreeDiskAsync<u32, u64>> = Arc::new(
            FractalTreeDiskAsync::from_buffer(filename, block_locations_pos)
                .await
                .unwrap(),
        );

        Ok(BytesBlockStore {
            block_locations,
            block_locations_pos,
            block_size,
            data: None,
            cache: Arc::new(AsyncLRU::new(16)), /* Small, because it's
                                                 * unlikely to reuse many
                                                 * blocks */
            compression_config,
            block_jobs: Arc::new(Mutex::new(Vec::new())),
        })
    }

    pub fn block_size(&self) -> usize
    {
        self.block_size as usize
    }

    // todo create a spsc channel to send the blocks in order, allowing
    // for prefetching
    #[cfg(feature = "async")]
    pub async fn block_queue(
        self: Arc<Self>,
        fhm: Arc<AsyncFileHandleManager>,
    ) -> Receiver<(u32, Bytes)>
    {
        // Number here is how many to prefetch
        const PREFETCH: usize = 8;
        let (tx, rx) = tokio::sync::mpsc::channel(PREFETCH);

        tokio::spawn(async move {
            let block_stream = self.stream(fhm).await;

            tokio::pin!(block_stream);

            while let Some((loc, block)) = block_stream.next().await {
                tx.send((loc, block)).await.unwrap();
            }
            return ();
        });

        rx
    }

    // todo iterator/generator for non-async version
    #[cfg(feature = "async")]
    pub async fn stream(
        self: Arc<Self>,
        fhm: Arc<AsyncFileHandleManager>,
    ) -> impl Stream<Item = (u32, Bytes)>
    {
        let gen = stream! {
            let block_locs = Arc::clone(&self.block_locations).stream().await;
            tokio::pin!(block_locs);

            // It's ok to hold the lock the entire time...
            let mut in_buf = fhm.get_filehandle().await;

            while let Some(loc) = block_locs.next().await {

                let block = self.get_block_pos_known(&mut in_buf, loc.0, loc.1).await;
                yield match block {
                    DataOrLater::Data(data) => (loc.0, data),
                    DataOrLater::Later(data) => (loc.0, data.await.unwrap()),
                };
            }
        };

        gen
    }
}

#[cfg(feature = "async")]
/// For storing blocks, based off of block number, so (block, Vec<u8>)
///
/// This is used for storing blocks in memory, so that we can
/// quickly access them without having to read from disk each time.
///
/// This is used in the async version of BytesBlockStore.
///
/// When size is exceeded, removes the least recently used block.
///
/// Has RwLock for thread safety (assuming tokio async)
struct AsyncLRU
{
    /// Maximum size of the LRU
    max_size: usize,

    /// The LRU itself, stored as vec so we can just iterate
    /// through...
    lru: Arc<RwLock<HashMap<u32, Bytes>>>,

    /// The order of the LRU
    order: Arc<RwLock<std::collections::VecDeque<u32>>>,
}

#[cfg(feature = "async")]
impl AsyncLRU
{
    /// Create a new LRU with a maximum size
    pub fn new(max_size: usize) -> Self
    {
        AsyncLRU {
            max_size,
            lru: Arc::new(RwLock::new(HashMap::new())),
            order: Arc::new(RwLock::new(std::collections::VecDeque::new())),
        }
    }

    /// Get a block from the LRU
    /// If not found, return None (another function handles adding to
    /// the cache)
    #[tracing::instrument(skip(self))]
    pub async fn get(&self, block: u32) -> Option<Bytes>
    {
        let cache = self.lru.read().await;
        let mut order = self.order.write().await;

        if let Some(data) = cache.get(&block) {
            order.retain(|&x| x != block);
            order.push_front(block);

            return Some(data.clone());
        }

        None
    }

    /// Add a block to the LRU
    #[tracing::instrument(skip(self, data))]
    pub async fn add(&self, block: u32, data: Bytes) -> Bytes
    {
        let mut cache = self.lru.write().await;
        let mut order = self.order.write().await;

        // If the block is already in the LRU, move it to the front
        if cache.contains_key(&block) {
            order.retain(|&x| x != block);
        } else {
            if cache.len() >= self.max_size {
                if let Some(lru_block) = order.pop_back() {
                    cache.remove(&lru_block);
                }
            }
        }

        order.push_front(block);
        cache.insert(block, data.clone());
        data
    }
}

#[cfg(feature = "async")]
pub struct BytesBlockStoreBlockReader
{
    active: Option<Receiver<(u32, Bytes)>>,
    current_block: (u32, Bytes),
}

#[cfg(feature = "async")]
impl BytesBlockStoreBlockReader
{
    pub async fn new(
        block_store: Arc<BytesBlockStore>,
        fhm: Arc<AsyncFileHandleManager>,
    ) -> Self
    {
        // let store = Arc::clone(&block_store);
        // let stream = store.stream(fhm).await;
        let mut stream = block_store.block_queue(fhm).await;

        // let mut boxed = Box::pin(stream);
        // let current_block = boxed.next().await.unwrap();

        let current_block = stream.recv().await.unwrap();

        BytesBlockStoreBlockReader {
            active: Some(stream),
            current_block,
        }
    }

    pub async fn next(&mut self, loc: &[Loc]) -> Option<BytesMut>
    {
        let locs = &loc[..];
        let mut results = Vec::new();

        let locs_len = locs.len();
        let mut locs_idx = 0;

        while locs_idx < locs_len {
            let loc = &locs[locs_idx];
            let block = loc.block;

            debug_assert!(
                block >= self.current_block.0,
                "Block: {} Cached: {}",
                block,
                self.current_block.0
            );

            if block == self.current_block.0 {
                let start = loc.start as usize;
                let end = (loc.start + loc.len) as usize;
                let j = self.current_block.1.slice(start..end);
                results.push(j);
                locs_idx += 1;
            } else {
                // let block =
                // self.active.as_mut().unwrap().next().await.unwrap();
                let block = self.active.as_mut().unwrap().recv().await.unwrap();
                self.current_block = block;
            }
        }

        if results.is_empty() {
            return None;
        }

        let mut result =
            BytesMut::with_capacity(results.iter().map(|r| r.len()).sum());
        for r in results {
            result.put(r);
        }

        Some(result)
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

    #[test]
    fn test_constructor()
    {
        let mut store = BytesBlockStoreBuilder::default();
        store.block_size = 10;
        store = store
            .with_dict()
            .with_dict_samples(128)
            .with_dict_size(64 * 1024);
        assert!(store.create_dict);
        assert!(store.dict_samples == 128);
        assert!(store.dict_size == 64 * 1024);
        assert!(store.block_size == 10);
    }
}
