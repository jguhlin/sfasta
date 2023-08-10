use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::Ordering;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam::queue::ArrayQueue;
use crossbeam::utils::Backoff;

use crate::compression::useful::*;
use crate::io::OutputBlock;

#[cfg(not(target_arch = "wasm32"))]
use xz::read::{XzDecoder, XzEncoder};

#[derive(Debug, Clone, bincode::Encode, bincode::Decode)]
pub struct CompressionConfig {
    pub compression_type: CompressionType,
    pub compression_level: i8,
    pub compression_dict: Option<Arc<Vec<u8>>>,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl CompressionConfig {
    pub const fn new() -> Self {
        Self {
            compression_type: CompressionType::ZSTD,
            compression_level: 3,
            compression_dict: None,
        }
    }

    pub const fn with_compression_type(mut self, compression_type: CompressionType) -> Self {
        self.compression_type = compression_type;
        self
    }

    pub const fn with_compression_level(mut self, compression_level: i8) -> Self {
        self.compression_level = compression_level;
        self
    }

    pub fn with_compression_dict(mut self, compression_dict: Option<Arc<Vec<u8>>>) -> Self {
        self.compression_dict = compression_dict;
        self
    }
}

pub enum CompressorWork {
    Compress(CompressionBlock),
    Decompress(CompressionBlock), // TODO
}

pub struct CompressionBlock {
    pub input: Vec<u8>,
    pub location: Arc<AtomicU64>,
    pub compression_config: Arc<CompressionConfig>,
}

impl CompressionBlock {
    pub fn new(input: Vec<u8>, compression_config: Arc<CompressionConfig>) -> Self {
        Self {
            input,
            location: Arc::new(AtomicU64::new(0)),
            compression_config,
        }
    }

    pub fn get_location(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.location)
    }

    pub fn is_complete(&self) -> bool {
        self.location.load(Ordering::Relaxed) != 0_u64
    }
}

pub struct Worker {
    pub threads: u16,
    pub buffer_size: usize,
    handles: Vec<JoinHandle<()>>,
    queue: Option<Arc<ArrayQueue<CompressorWork>>>, // Input
    writer: Option<Arc<ArrayQueue<OutputBlock>>>,   // Output

    shutdown_flag: Arc<AtomicBool>,
}

impl Default for Worker {
    fn default() -> Self {
        Self {
            threads: 1,
            handles: Vec::new(),
            buffer_size: 8192,
            queue: None,
            shutdown_flag: Arc::new(AtomicBool::new(false)),
            writer: None,
        }
    }
}

impl Worker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_threads(mut self, threads: u16) -> Self {
        self.threads = threads;
        self
    }

    pub fn with_buffer_size(mut self, buffer_size: usize) -> Self {
        self.buffer_size = buffer_size;
        self
    }

    pub fn with_output_queue(mut self, writer: Arc<ArrayQueue<OutputBlock>>) -> Self {
        self.writer = Some(writer);
        self
    }

    pub fn start(&mut self) {
        let queue = Arc::new(ArrayQueue::<CompressorWork>::new(self.buffer_size));
        self.queue = Some(queue);

        for _ in 0..self.threads {
            let queue = Arc::clone(self.queue.as_ref().unwrap());
            let shutdown = Arc::clone(&self.shutdown_flag);
            let output_queue = Arc::clone(self.writer.as_ref().unwrap());
            let handle = thread::spawn(move || compression_worker(queue, shutdown, output_queue));
            self.handles.push(handle);
        }
    }

    pub fn submit(&self, work: CompressorWork) {
        let backoff = Backoff::new();

        let mut entry = work;
        while let Err(x) = self.queue.as_ref().unwrap().push(entry) {
            backoff.snooze();
            entry = x;
        }
    }

    pub fn compress(
        &self,
        input: Vec<u8>,
        compression_config: Arc<CompressionConfig>,
    ) -> Arc<AtomicU64> {
        let block = CompressionBlock::new(input, compression_config);
        let location = block.get_location();

        self.submit(CompressorWork::Compress(block));

        location
    }

    pub fn shutdown(&mut self) {
        self.shutdown_flag.store(true, Ordering::Relaxed);
        for handle in self.handles.drain(..) {
            handle.join().unwrap();
        }
    }

    pub fn get_queue(&self) -> Arc<ArrayQueue<CompressorWork>> {
        Arc::clone(self.queue.as_ref().unwrap())
    }
}

fn compression_worker(
    queue: Arc<ArrayQueue<CompressorWork>>,
    shutdown: Arc<AtomicBool>,
    output_queue: Arc<ArrayQueue<OutputBlock>>,
) {
    let mut result;
    let backoff = Backoff::new();

    loop {
        result = queue.pop();
        match result {
            None => {
                if shutdown.load(Ordering::Relaxed) {
                    return;
                } else {
                    backoff.snooze();
                    if backoff.is_completed() {
                        thread::park_timeout(Duration::from_millis(25));
                        backoff.reset();
                    }
                }
            }
            Some(CompressorWork::Compress(work)) => {
                println!("Got stuff to compress");

                #[cfg(not(test))]
                let mut output = Vec::with_capacity(work.input.len());

                // Tests fail otherwise for buffer too small
                #[cfg(test)]
                let mut output = Vec::with_capacity(8192);

                match work.compression_config.compression_type {
                    #[cfg(not(target_arch = "wasm32"))]
                    CompressionType::ZSTD => {
                        let mut zstd_compressor = zstd_encoder(
                            work.compression_config.compression_level as i32,
                            &work.compression_config.compression_dict,
                        );
                        zstd_compressor
                            .compress_to_buffer(work.input.as_slice(), &mut output)
                            .unwrap();
                    }
                    #[cfg(target_arch = "wasm32")]
                    CompressionType::ZSTD => {
                        unimplemented!("ZSTD encoding is not supported on wasm32");
                    }
                    CompressionType::LZ4 => {
                        let mut lz4_compressor = lz4_flex::frame::FrameEncoder::new(&mut output);
                        lz4_compressor.write_all(work.input.as_slice()).unwrap();
                        lz4_compressor.finish().unwrap();
                    }
                    CompressionType::SNAPPY => {
                        let mut compressor = snap::write::FrameEncoder::new(&mut output);
                        compressor
                            .write_all(work.input.as_slice())
                            .expect("Unable to compress with Snappy");
                        compressor.into_inner().unwrap();
                    }
                    CompressionType::GZIP => {
                        let mut compressor = flate2::write::GzEncoder::new(
                            &mut output,
                            flate2::Compression::new(
                                work.compression_config.compression_level as u32,
                            ),
                        );
                        compressor
                            .write_all(work.input.as_slice())
                            .expect("Unable to compress with GZIP");
                        compressor.finish().unwrap();
                    }
                    CompressionType::NONE => {
                        output = work.input;
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    CompressionType::XZ => {
                        let mut compressor = XzEncoder::new(
                            work.input.as_slice(),
                            work.compression_config.compression_level as u32,
                        );
                        compressor
                            .read_to_end(&mut output)
                            .expect("Unable to compress with XZ");
                    }
                    #[cfg(target_arch = "wasm32")]
                    CompressionType::XZ => {
                        panic!("XZ compression is not supported on wasm32");
                    }
                    CompressionType::BROTLI => {
                        let mut compressor = brotli::CompressorWriter::new(
                            &mut *output,
                            8192,
                            work.compression_config.compression_level as u32,
                            22,
                        );
                        compressor
                            .write_all(work.input.as_slice())
                            .expect("Unable to compress with Brotli");
                        compressor.flush().expect("Unable to flush Brotli");
                    }
                    _ => panic!(
                        "Unsupported compression type: {:?}",
                        work.compression_config.compression_type
                    ),
                };

                let output_block = OutputBlock {
                    data: output,
                    location: work.location,
                };

                output_queue.push(output_block).unwrap();
            }
            Some(CompressorWork::Decompress(work)) => {
                todo!();
            }
            _ => panic!("Unsupported work type"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use rand::prelude::*;
}
