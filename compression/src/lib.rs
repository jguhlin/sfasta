extern crate core_affinity;

use bzip2::Compression;
use serde::{Deserialize, Serialize};

use std::{
    io::{Read, Write},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread,
    thread::JoinHandle,
    time::Duration,
};

use lz4_flex::block::{compress_prepend_size, decompress_size_prepended};

use liblzma::read::{XzDecoder, XzEncoder};

pub const MAX_DECOMPRESS_SIZE: usize = 1024 * 1024 * 1024; // 1GB

use std::{cell::RefCell, rc::Rc};

thread_local! {
    static ZSTD_COMPRESSOR: RefCell<zstd::bulk::Compressor<'static>> = RefCell::new(zstd_encoder(3, &None));
    static ZSTD_DECOMPRESSOR: RefCell<zstd::bulk::Decompressor<'static>> = RefCell::new(zstd_decompressor(None));

    // todo, all the others
}

use crossbeam::{queue::ArrayQueue, utils::Backoff};

#[derive(
    PartialEq,
    Eq,
    Debug,
    Clone,
    Copy,
    bincode::Encode,
    bincode::Decode,
    Serialize,
    Deserialize,
)]
#[non_exhaustive]
pub enum CompressionType
{
    ZSTD,   // 1 should be default compression ratio
    LZ4,    // 9 should be default compression ratio
    SNAPPY, // Implemented
    GZIP,   // Implemented
    NONE,   // No Compression
    XZ,     // Implemented, 6 is default ratio
    BROTLI, // Implemented, 6 is default
    BZIP2,  // Implemented
    BIT2,   // Not implemented
    BIT4,   // Not implemented
}

#[derive(Debug, Clone)]
pub struct OutputBlock
{
    pub data: Vec<u8>,
    pub location: Arc<AtomicU64>,
}

#[derive(
    Debug,
    Clone,
    bincode::Encode,
    bincode::Decode,
    Serialize,
    Deserialize,
    Eq,
    PartialEq,
)]
pub struct CompressionConfig
{
    pub compression_type: CompressionType,
    pub compression_level: i8,

    #[serde(skip)]
    pub compression_dict: Option<Vec<u8>>,
}

impl Default for CompressionConfig
{
    fn default() -> Self
    {
        Self::new()
    }
}

impl CompressionConfig
{
    pub const fn new() -> Self
    {
        Self {
            compression_type: CompressionType::ZSTD,
            compression_level: 3,
            compression_dict: None,
        }
    }

    pub const fn with_compression_type(
        mut self,
        compression_type: CompressionType,
    ) -> Self
    {
        self.compression_type = compression_type;
        self
    }

    pub fn with_compression_level(mut self, compression_level: i8) -> Self
    {
        self.compression_level = compression_level;
        match self.check_compression_level() {
            Ok(_) => (),
            Err(x) => {
                log::warn!(
                    "Compression level {} is out of range for {:?}. Setting to {}",
                    compression_level,
                    self.compression_type,
                    x
                );
                self.compression_level = x;
            }
        };
        self
    }

    pub fn with_compression_dict(
        mut self,
        compression_dict: Option<Vec<u8>>,
    ) -> Self
    {
        self.compression_dict = compression_dict;
        self
    }

    pub fn check_compression_level(&self) -> Result<(), i8>
    {
        let (min, max) = self.compression_type.compression_level_range();
        if self.compression_level < min {
            Err(min)
        } else if self.compression_level > max {
            Err(max)
        } else {
            Ok(())
        }
    }

    /// Only used by FractalTree's....
    /// need todo fix that
    pub fn compress(&self, bytes: &[u8]) -> Result<Vec<u8>, std::io::Error>
    {
        match self.compression_type {
            CompressionType::ZSTD => {
                if self.compression_dict.is_some() {
                    ZSTD_COMPRESSOR.with_borrow_mut(|zstd_compressor| {
                        *zstd_compressor = zstd_encoder(
                            self.compression_level as i32,
                            &self.compression_dict,
                        );
                        zstd_compressor.compress(bytes)
                    })
                } else {
                    // Use thread local
                    ZSTD_COMPRESSOR.with_borrow_mut(|zstd_compressor| {
                        *zstd_compressor =
                            zstd_encoder(self.compression_level as i32, &None);
                        zstd_compressor.compress(bytes)
                    })
                }
            }
            CompressionType::LZ4 => Ok(compress_prepend_size(&bytes)),
            CompressionType::NONE => {
                log::debug!("Compress called for none, which involes copying bytes. Prefer not to use it!");
                Ok(bytes.to_vec())
            }
            CompressionType::BROTLI => {
                let mut output = Vec::with_capacity(bytes.len());
                let mut compressor = brotli::CompressorWriter::new(
                    &mut output,
                    8192,
                    self.compression_level as u32,
                    22,
                );
                compressor
                    .write_all(bytes)
                    .expect("Unable to compress with Brotli");
                compressor.flush().expect("Unable to flush Brotli");
                drop(compressor);
                Ok(output)
            }
            CompressionType::XZ => {
                let mut output = Vec::with_capacity(bytes.len());
                let mut compressor =
                    XzEncoder::new(bytes, self.compression_level as u32);
                compressor
                    .read_to_end(&mut output)
                    .expect("Unable to compress with XZ");
                Ok(output)
            }
            CompressionType::SNAPPY => {
                let mut output = Vec::with_capacity(bytes.len());
                let mut compressor =
                    snap::write::FrameEncoder::new(&mut output);
                compressor
                    .write_all(bytes)
                    .expect("Unable to compress with Snappy");
                compressor.into_inner().unwrap();
                Ok(output)
            }
            CompressionType::GZIP => {
                let mut output = Vec::with_capacity(bytes.len());
                let mut compressor = flate2::write::GzEncoder::new(
                    &mut output,
                    flate2::Compression::new(self.compression_level as u32),
                );
                compressor
                    .write_all(bytes)
                    .expect("Unable to compress with GZIP");
                compressor.finish().unwrap();
                Ok(output)
            }
            CompressionType::BZIP2 => {
                let mut compressor = bzip2::Compress::new(
                    bzip2::Compression::new(self.compression_level as u32),
                    0,
                );

                let mut output = Vec::with_capacity(bytes.len() * 8);

                compressor
                    .compress(bytes, &mut output, bzip2::Action::Finish)
                    .unwrap();
                output.shrink_to_fit();

                Ok(output)
            }
            _ => {
                // Todo: implement others. This isn't just used for
                // fractaltrees...
                panic!(
                    "Unsupported compression type: {:?}",
                    self.compression_type
                );
            }
        }
    }

    pub fn decompress(&self, bytes: &[u8]) -> Result<Vec<u8>, std::io::Error>
    {
        match self.compression_type {
            CompressionType::ZSTD => {
                ZSTD_DECOMPRESSOR.with_borrow_mut(|zstd_decompressor| {
                    zstd_decompressor.decompress(bytes, MAX_DECOMPRESS_SIZE)
                })
            }
            CompressionType::LZ4 => {
                Ok(decompress_size_prepended(bytes).unwrap())
            }
            CompressionType::NONE => {
                log::debug!("Decompress called for none, which involes copying bytes. Prefer not to use it!");
                Ok(bytes.to_vec())
            }
            CompressionType::BROTLI => {
                let mut output = Vec::with_capacity(bytes.len());
                let mut decompressor =
                    brotli::Decompressor::new(bytes, bytes.len());
                decompressor
                    .read_to_end(&mut output)
                    .expect("Unable to decompress with Brotli");
                Ok(output)
            }
            CompressionType::XZ => {
                let mut output = Vec::with_capacity(bytes.len());
                let mut decompressor = XzDecoder::new(bytes);
                decompressor
                    .read_to_end(&mut output)
                    .expect("Unable to decompress with XZ");
                Ok(output)
            }
            CompressionType::SNAPPY => {
                let mut output = Vec::with_capacity(bytes.len());
                let mut decompressor = snap::read::FrameDecoder::new(bytes);
                decompressor
                    .read_to_end(&mut output)
                    .expect("Unable to decompress with Snappy");
                Ok(output)
            }
            CompressionType::GZIP => {
                let mut output = Vec::with_capacity(bytes.len());
                let mut decompressor = flate2::read::GzDecoder::new(bytes);
                decompressor
                    .read_to_end(&mut output)
                    .expect("Unable to decompress with GZIP");
                Ok(output)
            }
            CompressionType::BZIP2 => {
                let mut decompressor = bzip2::Decompress::new(false);
                let mut output = Vec::with_capacity(bytes.len() * 8);
                decompressor.decompress(bytes, &mut output).unwrap();
                Ok(output)
            }
            _ => {
                panic!(
                    "Unsupported compression type: {:?}",
                    self.compression_type
                );
            }
        }
    }
}

impl CompressionType
{
    pub const fn default_compression_level(&self) -> i8
    {
        match self {
            CompressionType::ZSTD => 3,
            CompressionType::LZ4 => 0, // Not applicable
            CompressionType::SNAPPY => 0, // Not applicable
            CompressionType::GZIP => 6,
            CompressionType::NONE => 0,
            CompressionType::XZ => 6,
            CompressionType::BROTLI => 9,
            CompressionType::BZIP2 => 6,
            _ => panic!("Unsupported compression type"),
        }
    }

    pub const fn compression_level_range(&self) -> (i8, i8)
    {
        match self {
            CompressionType::ZSTD => (1, 22),
            CompressionType::LZ4 => (0, 0),
            CompressionType::SNAPPY => (0, 0),
            CompressionType::GZIP => (0, 9),
            CompressionType::NONE => (0, 0),
            CompressionType::XZ => (0, 9),
            CompressionType::BROTLI => (0, 11),
            CompressionType::BZIP2 => (0, 9),
            _ => panic!("Unsupported compression type"),
        }
    }
}

impl Default for CompressionType
{
    fn default() -> CompressionType
    {
        CompressionType::ZSTD
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn zstd_encoder(
    compression_level: i32,
    dict: &Option<Vec<u8>>,
) -> zstd::bulk::Compressor<'static>
{
    let mut encoder = if let Some(dict) = dict {
        zstd::bulk::Compressor::with_dictionary(compression_level, &dict)
            .unwrap()
    } else {
        zstd::bulk::Compressor::new(compression_level).unwrap()
    };
    encoder.include_checksum(false).unwrap();
    encoder
        .include_magicbytes(false)
        .expect("Unable to set ZSTD MagicBytes");
    encoder
        .include_contentsize(false)
        .expect("Unable to set ZSTD Content Size Flag");
    encoder
        .long_distance_matching(true)
        .expect("Unable to set long_distance_matching");
    encoder.window_log(31).expect("Unable to set window_log");
    encoder
}

pub fn change_compression_level(compression_level: i32, dict: &Option<Vec<u8>>)
{
    ZSTD_COMPRESSOR.with(|zstd_compressor| {
        let encoder = zstd_encoder(compression_level, dict);
        *zstd_compressor.borrow_mut() = encoder;
    });
}

pub fn zstd_decompressor<'a>(
    dict: Option<&[u8]>,
) -> zstd::bulk::Decompressor<'a>
{
    let mut zstd_decompressor = if let Some(dict) = dict {
        zstd::bulk::Decompressor::with_dictionary(dict)
    } else {
        zstd::bulk::Decompressor::new()
    }
    .unwrap();

    zstd_decompressor
        .include_magicbytes(false)
        .expect("Failed to set magicbytes");

    // zstd_decompressor.window_log_max(31);

    zstd_decompressor
}

pub enum CompressorWork
{
    Compress(CompressionBlock),
    Decompress(CompressionBlock), // TODO
}

pub struct CompressionBlock
{
    pub input: Vec<u8>,
    pub location: Arc<AtomicU64>,
    pub compression_config: Arc<CompressionConfig>,
}

impl CompressionBlock
{
    pub fn new(
        input: Vec<u8>,
        compression_config: Arc<CompressionConfig>,
    ) -> Self
    {
        Self {
            input,
            location: Arc::new(AtomicU64::new(0)),
            compression_config,
        }
    }

    pub fn get_location(&self) -> Arc<AtomicU64>
    {
        Arc::clone(&self.location)
    }

    pub fn is_complete(&self) -> bool
    {
        self.location.load(Ordering::Relaxed) != 0_u64
    }
}

pub struct CompressionWorker
{
    pub threads: u16,
    pub buffer_size: usize,
    handles: Vec<JoinHandle<()>>,
    queue: Option<Arc<ArrayQueue<CompressorWork>>>, // Input
    writer: Option<Arc<flume::Sender<OutputBlock>>>, // Output

    shutdown_flag: Arc<AtomicBool>,
}

impl Default for CompressionWorker
{
    fn default() -> Self
    {
        Self {
            threads: 1,
            handles: Vec::new(),
            buffer_size: 128,
            queue: None,
            shutdown_flag: Arc::new(AtomicBool::new(false)),
            writer: None,
        }
    }
}

impl CompressionWorker
{
    pub fn new() -> Self
    {
        Self::default()
    }

    pub fn with_threads(mut self, threads: u16) -> Self
    {
        self.threads = threads;
        self
    }

    pub fn with_buffer_size(mut self, buffer_size: usize) -> Self
    {
        self.buffer_size = buffer_size;
        self
    }

    pub fn with_output_queue(
        mut self,
        writer: Arc<flume::Sender<OutputBlock>>,
    ) -> Self
    {
        self.writer = Some(writer);
        self
    }

    pub fn start(&mut self)
    {
        let queue =
            Arc::new(ArrayQueue::<CompressorWork>::new(self.buffer_size));
        self.queue = Some(queue);

        let mut core_ids = core_affinity::get_core_ids().unwrap();

        if self.threads as usize >= core_ids.len() {
            log::debug!(
                "Detected {} Physical Cores. Setting threads to {}",
                core_ids.len(),
                self.threads
            );

            self.threads = core_ids.len() as u16;
        }

        for _ in 0..self.threads {
            let id = core_ids.pop().unwrap();

            let queue = Arc::clone(self.queue.as_ref().unwrap());
            let shutdown = Arc::clone(&self.shutdown_flag);
            let output_queue = Arc::clone(self.writer.as_ref().unwrap());
            let handle = thread::spawn(move || {
                // NOTE: This shaves off a few seconds on my FASTQ example, so
                // keeping it. Could be more for even larger files, and won't
                // affect much for small files
                let res = core_affinity::set_for_current(id);
                if res {
                    compression_worker(queue, shutdown, output_queue)
                }
            });
            self.handles.push(handle);
        }
    }

    #[inline]
    pub fn submit(&self, work: CompressorWork)
    {
        let backoff = Backoff::new();

        let mut entry = work;
        while let Err(x) = self.queue.as_ref().unwrap().push(entry) {
            backoff.snooze();
            if backoff.is_completed() {
                thread::park_timeout(Duration::from_millis(10));
                backoff.reset();
            }
            entry = x;
        }
    }

    #[inline]
    pub fn compress(
        &self,
        input: Vec<u8>,
        compression_config: Arc<CompressionConfig>,
    ) -> Arc<AtomicU64>
    {
        let block = CompressionBlock::new(input, compression_config);
        let location = block.get_location();

        self.submit(CompressorWork::Compress(block));

        location
    }

    pub fn shutdown(&mut self)
    {
        self.shutdown_flag.store(true, Ordering::Relaxed);
        for handle in self.handles.drain(..) {
            handle.join().unwrap();
        }
    }

    #[inline]
    pub fn get_queue(&self) -> Arc<ArrayQueue<CompressorWork>>
    {
        Arc::clone(self.queue.as_ref().unwrap())
    }

    #[inline]
    pub fn len(&self) -> usize
    {
        self.queue.as_ref().unwrap().len()
    }
}

fn compression_worker(
    queue: Arc<ArrayQueue<CompressorWork>>,
    shutdown: Arc<AtomicBool>,
    output_queue: Arc<flume::Sender<OutputBlock>>,
)
{
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
                        thread::park_timeout(Duration::from_millis(1));
                        backoff.reset();
                    }
                }
            }
            Some(CompressorWork::Compress(work)) => {
                // Tests fail otherwise for buffer too small
                let compressed = match work.compression_config.compression_type
                {
                    CompressionType::ZSTD => {
                        ZSTD_COMPRESSOR.with_borrow_mut(|zstd_compressor| {
                            zstd_compressor
                                .set_compression_level(
                                    work.compression_config.compression_level
                                        as i32,
                                )
                                .unwrap();

                            if work
                                .compression_config
                                .compression_dict
                                .is_some()
                            {
                                zstd_compressor
                                    .set_dictionary(
                                        work.compression_config
                                            .compression_level
                                            as i32,
                                        work.compression_config
                                            .compression_dict
                                            .as_ref()
                                            .unwrap()
                                            .as_ref(),
                                    )
                                    .unwrap();
                            } else {
                                // Annoying.. todo
                                // todo store in hashmap so that one encoder is
                                // built for each compression config already?
                                *zstd_compressor = zstd_encoder(
                                    work.compression_config.compression_level
                                        as i32,
                                    &None,
                                );
                            }
                            zstd_compressor
                                .compress(work.input.as_slice())
                                .unwrap()
                        })
                    }
                    CompressionType::LZ4 => {
                        let mut output = Vec::with_capacity(work.input.len());
                        let mut lz4_compressor =
                            lz4_flex::frame::FrameEncoder::new(&mut output);
                        lz4_compressor
                            .write_all(work.input.as_slice())
                            .unwrap();
                        lz4_compressor.finish().unwrap();
                        output
                    }
                    CompressionType::SNAPPY => {
                        let mut output = Vec::with_capacity(work.input.len());
                        let mut compressor =
                            snap::write::FrameEncoder::new(&mut output);
                        compressor
                            .write_all(work.input.as_slice())
                            .expect("Unable to compress with Snappy");
                        compressor.into_inner().unwrap();
                        output
                    }
                    CompressionType::GZIP => {
                        let mut output = Vec::with_capacity(work.input.len());
                        let mut compressor = flate2::write::GzEncoder::new(
                            &mut output,
                            flate2::Compression::new(
                                work.compression_config.compression_level
                                    as u32,
                            ),
                        );
                        compressor
                            .write_all(work.input.as_slice())
                            .expect("Unable to compress with GZIP");
                        compressor.finish().unwrap();
                        output
                    }
                    CompressionType::NONE => work.input,
                    CompressionType::XZ => {
                        let mut output = Vec::with_capacity(work.input.len());
                        let mut compressor = XzEncoder::new(
                            work.input.as_slice(),
                            work.compression_config.compression_level as u32,
                        );
                        compressor
                            .read_to_end(&mut output)
                            .expect("Unable to compress with XZ");
                        output
                    }
                    CompressionType::BROTLI => {
                        let mut output = Vec::with_capacity(work.input.len());
                        let mut compressor = brotli::CompressorWriter::new(
                            &mut output,
                            8192,
                            work.compression_config.compression_level as u32,
                            22,
                        );
                        compressor
                            .write_all(work.input.as_slice())
                            .expect("Unable to compress with Brotli");
                        compressor.flush().expect("Unable to flush Brotli");
                        drop(compressor);
                        output
                    }
                    CompressionType::BZIP2 => {
                        let mut compressor = bzip2::Compress::new(
                            bzip2::Compression::new(
                                work.compression_config.compression_level
                                    as u32,
                            ),
                            0,
                        );

                        let mut output =
                            Vec::with_capacity(work.input.len() * 8);

                        compressor
                            .compress(
                                work.input.as_slice(),
                                &mut output,
                                bzip2::Action::Finish,
                            )
                            .unwrap();
                        output.shrink_to_fit();

                        output
                    }
                    _ => panic!(
                        "Unsupported compression type: {:?}",
                        work.compression_config.compression_type
                    ),
                };

                let output_block = OutputBlock {
                    data: compressed,
                    location: work.location,
                };

                let mut x = output_queue.try_send(output_block);
                while let Err(y) = x {
                    log::trace!("Output queue is full");
                    backoff.snooze();
                    match y {
                        flume::TrySendError::Full(y) => {
                            x = output_queue.try_send(y);
                        }
                        flume::TrySendError::Disconnected(_) => {
                            panic!("Output queue disconnected");
                        }
                    }
                }
            }

            Some(CompressorWork::Decompress(work)) => {
                todo!();
            }
            _ => panic!("Unsupported work type"),
        }
    }
}

// todo cute nucleotides is better
pub fn can_store_2bit(bytes: &[u8]) -> bool {
    // ACTG, but no N, and never protein
    // can also be lower case (masking stored separately)
    bytes.iter().all(|&x| {
        x == b'A'
            || x == b'C'
            || x == b'G'
            || x == b'T'
            || x == b'a'
            || x == b'c'
            || x == b'g'
            || x == b't'
    })
}

// Convert to 2bit encoding
pub fn to_2bit(bytes: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(bytes.len() / 4);
    let mut buffer = 0_u8;
    let mut shift = 6_u8;
    for &x in bytes.iter() {
        let val = match x {
            b'A' | b'a' => 0,
            b'C' | b'c' => 1,
            b'G' | b'g' => 2,
            b'T' | b't' => 3,
            _ => panic!("Invalid 2bit character"),
        };
        buffer |= val << shift;
        shift -= 2;
        if shift == 0 {
            output.push(buffer);
            buffer = 0;
            shift = 6;
        }
    }
    if shift != 6 {
        output.push(buffer);
    }
    output
}

// Convert from 2bit encoding
pub fn from_2bit(bytes: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(bytes.len() * 4);
    for &x in bytes.iter() {
        let mut shift = 6;
        while shift != 0 {
            let val = (x >> shift) & 0b11;
            let val = match val {
                0 => b'A',
                1 => b'C',
                2 => b'G',
                3 => b'T',
                _ => panic!("Invalid 2bit character"),
            };
            output.push(val);
            shift -= 2;
        }
    }
    output
}

#[cfg(test)]
mod tests
{
    use super::*;

    #[test]
    pub fn test_out_of_range()
    {
        let compression_config = CompressionConfig::new();
        let compression_config =
            compression_config.with_compression_type(CompressionType::ZSTD);
        let compression_config = compression_config.with_compression_level(23);
        assert_eq!(compression_config.compression_level, 22);

        // BROTLI
        let compression_config = CompressionConfig::new();
        let compression_config =
            compression_config.with_compression_type(CompressionType::BROTLI);
        let compression_config = compression_config.with_compression_level(12);
        assert_eq!(compression_config.compression_level, 11);
    }
}
