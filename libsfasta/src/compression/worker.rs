use std::collections::HashMap;
use std::io::{Write, Read};
use std::sync::atomic::Ordering;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::{Arc, Mutex};
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam::queue::ArrayQueue;
use crossbeam::utils::Backoff;

#[cfg(not(target_arch = "wasm32"))]
use xz::read::{XzDecoder, XzEncoder};

use crate::datatypes::*;
use crate::CompressionType;

pub enum CompressionWorkerOrder {
    Compress(CompressionWork),
    // Decompress(CompressionWork), // todo
    // Shutdown
}
pub struct CompressionWork {
    pub input: Arc<Vec<u8>>,
    pub output: Arc<Mutex<Vec<u8>>>,
    pub compression_type: CompressionType,
    pub compression_level: i8,
    pub compression_dict: Option<Arc<Vec<u8>>>,
}

pub struct Worker {
    pub threads: u16,
    pub buffer_size: usize,
    handles: Vec<JoinHandle<()>>,
    queue: Option<Arc<ArrayQueue<CompressionWorkerOrder>>>,
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

    pub fn start(&mut self) {
        let queue = Arc::new(ArrayQueue::<CompressionWorkerOrder>::new(self.buffer_size));
        self.queue = Some(queue);

        for _ in 0..self.threads {
            let queue = Arc::clone(self.queue.as_ref().unwrap());
            let shutdown = Arc::clone(&self.shutdown_flag);
            let handle = thread::spawn(move || compression_worker(queue, shutdown));
            self.handles.push(handle);
        }

    }

    pub fn submit(&self, work: CompressionWorkerOrder) {
        self.queue.as_ref().unwrap().push(work);
    }

    pub fn shutdown(&mut self) {
        self.shutdown_flag.store(true, Ordering::Relaxed);
        for handle in self.handles.drain(..) {
            handle.join().unwrap();
        }
    }
}

fn compression_worker(
    queue: Arc<ArrayQueue<CompressionWorkerOrder>>,
    shutdown: Arc<AtomicBool>,
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
                        thread::park_timeout(Duration::from_millis(100));
                        backoff.reset();
                    }
                }
            }
            Some(CompressionWorkerOrder::Compress(work)) => {
                let mut output = work.output.lock().unwrap();

                match work.compression_type {
                    #[cfg(not(target_arch = "wasm32"))]
                    CompressionType::ZSTD => {
                        let mut zstd_compressor =
                            zstd_encoder(work.compression_level as i32, work.compression_dict);
                        zstd_compressor
                            .compress_to_buffer(work.input.as_slice(), &mut *output)
                            .unwrap();
                    }
                    #[cfg(target_arch = "wasm32")]
                    CompressionType::ZSTD => {
                        unimplemented!("ZSTD encoding is not supported on wasm32");
                    }
                    CompressionType::LZ4 => {
                        let mut lz4_compressor = lz4_flex::frame::FrameEncoder::new(&mut *output);
                        lz4_compressor.write_all(work.input.as_slice()).unwrap();
                        lz4_compressor.finish().unwrap();
                    }
                    CompressionType::SNAPPY => {
                        let mut compressor = snap::write::FrameEncoder::new(&mut *output);
                        compressor
                            .write_all(work.input.as_slice())
                            .expect("Unable to compress with Snappy");
                        compressor.into_inner().unwrap();
                    }
                    CompressionType::GZIP => {
                        let mut compressor = flate2::write::GzEncoder::new(
                            &mut *output,
                            flate2::Compression::new(work.compression_level as u32),
                        );
                        compressor
                            .write_all(work.input.as_slice())
                            .expect("Unable to compress with GZIP");
                        compressor.finish().unwrap();
                    }
                    CompressionType::NONE => {
                        *output = work.input.as_slice().to_vec();
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    CompressionType::XZ => {
                        let mut compressor = XzEncoder::new(work.input.as_slice(), work.compression_level as u32);
                        compressor
                        .read_to_end(&mut *output)
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
                            work.compression_level as u32,
                            22,
                        );
                        compressor
                            .write_all(work.input.as_slice())
                            .expect("Unable to compress with Brotli");
                        compressor.flush().expect("Unable to flush Brotli");
                    }
                    _ => panic!("Unsupported compression type: {:?}", work.compression_type),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::prelude::*;

    #[test]
    pub fn test_compression() {
        let mut worker = Worker::new().with_threads(4).with_buffer_size(100);
        worker.start();

        // Create 8192 bytes of random data (ascii)
        let mut input = vec![b'A'; 8192];

        // Create 1000 places to store the compressed data
        let mut outputs = vec![Arc::new(Mutex::new(Vec::with_capacity(8192*8))); 1000];

        let input = Arc::new(input);

        for compression_type in vec![
            CompressionType::ZSTD,
            CompressionType::LZ4,
            CompressionType::SNAPPY,
            CompressionType::GZIP,
            CompressionType::NONE,
            CompressionType::XZ,
            CompressionType::BROTLI,
        ] {
            for output in outputs.iter_mut() {
                // Clear output
                let mut output_clear = output.lock().unwrap();
                output_clear.clear();
                drop(output_clear);

                let output = Arc::clone(output);
                let input = Arc::clone(&input);
                worker.submit(CompressionWorkerOrder::Compress(CompressionWork {
                    input,
                    output,
                    compression_type: compression_type,
                    compression_level: 3,
                    compression_dict: None,
                }));
            }
        }

        worker.shutdown();

        assert!(outputs.iter().all(|output| {
            let output = output.lock().unwrap();
            output.len() > 0
        }));

        let mut worker = Worker::new().with_threads(4).with_buffer_size(100);
        worker.start();

        for output in outputs.iter_mut() {
            let output = Arc::clone(output);
            let input = Arc::clone(&input);
            worker.submit(CompressionWorkerOrder::Compress(CompressionWork {
                input,
                output,
                compression_type: CompressionType::ZSTD,
                compression_level: 3,
                compression_dict: None,
            }));
        }

        worker.shutdown();

        // Decompress first output
        let mut decompressed = Vec::new();
        let output = outputs[0].lock().unwrap();
        let mut decoder = zstd::stream::read::Decoder::new(&output[..]).unwrap();
        decoder.include_magicbytes(false).expect("Unable to disable magic bytes");
        decoder.read_to_end(&mut decompressed).unwrap();

        assert_eq!(decompressed, *input);
        println!("Decompressed: {}", String::from_utf8_lossy(&decompressed));

    }
}
