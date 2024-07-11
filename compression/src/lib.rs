extern crate core_affinity;

use bzip2::Compression;
use serde::{Deserialize, Serialize};
use ux::{u3, u5};
// use htscodecs_sys::{fqz_compress, fqz_gparams, fqz_slice};
use bitm::{BitAccess, BitVec};
use crossbeam::{queue::ArrayQueue, utils::Backoff};
use liblzma::read::{XzDecoder, XzEncoder};
use lz4_flex::block::{compress_prepend_size, decompress_size_prepended};
use minimum_redundancy::{
    BitsPerFragment, Code, Coding, DecodingResult, Frequencies,
};

use std::{
    cell::RefCell,
    collections::HashMap,
    hash::Hash,
    io::{Read, Write},
    rc::Rc,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread,
    thread::JoinHandle,
    time::Duration,
};

// pub mod rans_impl;
// pub use rans_impl::*;

pub mod stream_based;
pub use stream_based::*;

pub const MAX_DECOMPRESS_SIZE: usize = 1024 * 1024 * 1024; // 1GB

thread_local! {
    static ZSTD_COMPRESSOR: RefCell<zstd::bulk::Compressor<'static>> = RefCell::new(zstd_encoder(3, &None));
    static ZSTD_DECOMPRESSOR: RefCell<zstd::bulk::Decompressor<'static>> = RefCell::new(zstd_decompressor(None));

    // todo, all the others
}

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
    ZSTD,   // 1 should be default compression level
    LZ4,    // 9 should be default compression level
    SNAPPY, // Implemented
    GZIP,   // Implemented
    NONE,   // No Compression
    XZ,     // Implemented, 6 is default level
    BROTLI, // Implemented, 6 is default
    BZIP2,  // Implemented
    BIT2,   // Not implemented
    BIT4,   // Not implemented
    MINIMUMREDUNDANCY,
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

            // Some improvement with zstd, not a ton
            // also this project isn't about finding/making new compression algorithms
            CompressionType::MINIMUMREDUNDANCY => {
                #[cfg(not(debug))]
                unimplemented!("Minimum Redundancy is not implemented in release mode");

                // Split into chunks
                let chunks = bytes.chunks_exact(3);
                // Get remainder early
                let remainder = chunks.remainder();

                // Copy into new vec
                let mut to_huffman = chunks.map(|x| x.to_vec()).collect::<Vec<Vec<u8>>>();
                // If there is a remainder, add it
                if !remainder.is_empty() {
                    to_huffman.push(remainder.to_vec());
                }

                // Calculate entropy
                let freqs = frequencies(&to_huffman);
                let degree = minimum_redundancy::entropy_to_bpf(freqs.entropy());
                println!("Degree: {}", degree);

                // let huffman: Coding<&[u8]> = Coding::from_iter(BitsPerFragment(degree), to_huffman.iter());
                let huffman = Coding::from_frequencies_cloned(BitsPerFragment(degree), &freqs);
                let size = total_size_bits(&freqs, &huffman.codes_for_values()) * degree as usize;

                let mut compressed = Box::<[u64]>::with_zeroed_bits(size);

                // println!("Degree: {}", degree);
                // println!("Size: {}", size);

                // Now encode
                // Get K/V pairs
                let dict = huffman.codes_for_values();
                let mut bit_index = 0;
                for k in to_huffman.iter() {
                    let c = dict[k];
                    // println!("Content: {:b} Len: {}", c.content, c.len);
                    // println!("Setting bits {} to {}", bit_index, (bit_index + c.len as usize * degree as usize));
                    // Currently set to:
                    // println!("Currently set to: {:b}", compressed.get_bits(bit_index, c.len as u8));

                    compressed.init_bits(bit_index, c.content as u64, c.len.min(32) as u8);
                    bit_index += c.len as usize * degree as usize;
                }

                let output: Vec<u64> = compressed.to_vec();

                let bincode_config = bincode::config::standard().with_variable_int_encoding();

                // Encoder
                let mut encoder_dict = Vec::new();
                // println!("Total count of values: {}", huffman.values.len());
                // println!("{:#?}", huffman.values);
                let values = bincode::encode_to_vec(&huffman.values, bincode_config).unwrap();

                // println!("Values len: {}", values.len());
                // println!("Internal Nodes Count: {:?}", huffman.internal_nodes_count);
                let internal_nodes_count = bincode::encode_to_vec(&huffman.internal_nodes_count, bincode_config).unwrap();
                let degree = bincode::encode_to_vec(&huffman.degree.0, bincode_config).unwrap();

                encoder_dict.extend(values);
                encoder_dict.extend(internal_nodes_count);
                encoder_dict.extend(degree);

                // Convert u64 to underlying bytes (u8)
                let data = output.iter().flat_map(|&x| x.to_be_bytes().to_vec()).collect::<Vec<u8>>();

                let mut output = Vec::new();
                output.extend(encoder_dict);
                output.extend(data);

                Ok(output)
            }
            /*
            CompressionType::FQZCOMP => {
                let uncomp_size = bytes.len();
                // Pointer for compressed size as size_t
                let mut comp_size: usize = 0;
                let vers = 2;

                /*
                pub struct fqz_slice
                    {
                    pub num_records: ::std::os::raw::c_int,
                    pub len: *mut u32,
                    pub flags: *mut u32,
                    } */

                // Treat it as a single slice, of bytes.len(), with no flags
                let mut s = fqz_slice {
                    num_records: 1,
                    len: std::ptr::null_mut(),
                    flags: std::ptr::null_mut(),
                };

                s.len = &uncomp_size as *const _ as *mut u32;
                s.flags = std::ptr::null_mut();

                let mut bytes = bytes.to_vec();

                let output = unsafe {
                    fqz_compress(vers, &mut s, bytes.as_mut_ptr() as *mut i8, uncomp_size as usize, &mut comp_size, 0, std::ptr::null_mut())
                };

                // Convert output to Vec<u8>
                let output = unsafe {
                    std::slice::from_raw_parts(output as *const u8, comp_size as usize)
                        .to_vec()
                };

                Ok(output)
            } */
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
        .include_dictid(false)
        .expect("Unable to set include_dictid");
    encoder
        .set_parameter(zstd::stream::raw::CParameter::BlockDelimiters(false))
        .expect("Unable to set max memory");

    encoder
        .long_distance_matching(true)
        .expect("Unable to set long_distance_matching");

    encoder.window_log(31).expect("Unable to set window_log");

    encoder
        .set_parameter(zstd::stream::raw::CParameter::HashLog(30))
        .expect("Unable to set max memory");
    // chain log
    encoder
        .set_parameter(zstd::stream::raw::CParameter::ChainLog(30))
        .expect("Unable to set max memory");

    // Search log
    encoder
        .set_parameter(zstd::stream::raw::CParameter::SearchLog(30))
        .expect("Unable to set max memory");

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
    pub compressed_size: Arc<AtomicU64>,
    pub decompressed_size: u64,
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
            decompressed_size: input.len() as u64,
            input,
            location: Arc::new(AtomicU64::new(0)),
            compression_config,
            compressed_size: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn get_location(&self) -> Arc<AtomicU64>
    {
        Arc::clone(&self.location)
    }

    pub fn get_compressed_size(&self) -> Arc<AtomicU64>
    {
        Arc::clone(&self.compressed_size)
    }

    pub fn is_complete(&self) -> bool
    {
        self.location.load(Ordering::Relaxed) != 0_u64
    }
}

fn total_size_bits<T: Eq + Hash + Clone>(
    frequencies: &HashMap<T, usize>,
    dict: &HashMap<T, Code>,
) -> usize
{
    frequencies
        .iter()
        .fold(0, |acc, (k, w)| acc + dict[&k].len as usize * *w)
}

fn frequencies<T: Eq + Hash + Clone>(data: &[T]) -> HashMap<T, usize>
{
    let result = HashMap::<T, usize>::with_occurrences_of(data.iter());
    result
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
    ) -> (Arc<AtomicU64>, Arc<AtomicU64>)
    {
        let block = CompressionBlock::new(input, compression_config);
        let location = block.get_location();
        let size = block.get_compressed_size();

        self.submit(CompressorWork::Compress(block));

        (location, size)
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
                    CompressionType::MINIMUMREDUNDANCY => {
                        #[cfg(not(debug))]
                        unimplemented!("Minimum Redundancy is not implemented in release mode");

                        let mut compression_config = CompressionConfig::new();
                        compression_config.compression_type =
                            CompressionType::MINIMUMREDUNDANCY;

                        let huffman_compressed =
                            compression_config.compress(&work.input).unwrap();
                        let compression_config = CompressionConfig::new();
                        let compression_config = compression_config
                            .with_compression_type(CompressionType::ZSTD);
                        let compression_config =
                            compression_config.with_compression_level(7);
                        let compressed = compression_config
                            .compress(&huffman_compressed)
                            .unwrap();
                        compressed
                    }
                    _ => panic!(
                        "Unsupported compression type: {:?}",
                        work.compression_config.compression_type
                    ),
                };

                // Set the compressed size
                work.compressed_size
                    .store(compressed.len() as u64, Ordering::Relaxed);

                let output_block = OutputBlock {
                    data: compressed,
                    location: work.location,
                };

                let mut x = output_queue.try_send(output_block);
                while let Err(y) = x {
                    // log::trace!("Output queue is full");
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
pub fn can_store_2bit(bytes: &[u8]) -> bool
{
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
pub fn to_2bit(bytes: &[u8]) -> Vec<u8>
{
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
pub fn from_2bit(bytes: &[u8]) -> Vec<u8>
{
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

// to 4 bit
// includes N
pub fn to_4bit(seq: &mut [u8])
{
    for x in seq.iter_mut() {
        match x {
            b'A' | b'a' => *x = 0,
            b'C' | b'c' => *x = 1,
            b'G' | b'g' => *x = 2,
            b'T' | b't' => *x = 3,
            b'N' | b'n' => *x = 4,
            _ => panic!("Invalid 4bit character"),
        }
    }
}

// from 4 bit
pub fn from_4bit(seq: &mut [u8])
{
    for x in seq.iter_mut() {
        match x {
            0 => *x = b'A',
            1 => *x = b'C',
            2 => *x = b'G',
            3 => *x = b'T',
            4 => *x = b'N',
            _ => panic!("Invalid 4bit character"),
        }
    }
}

#[cfg(test)]
mod tests
{

    use super::*;

    // use rans::RansDecoderMulti;
    //
    // This whole section is me figuring out how rANS works
    // htscodecs has it implemented, but I still want to learn how it
    // works #[test]
    // pub fn rans()
    // {
    // use rans::{
    // byte_decoder::{ByteRansDecSymbol, ByteRansDecoder},
    // byte_encoder::{ByteRansEncSymbol, ByteRansEncoder},
    // RansDecSymbol, RansDecoder, RansEncSymbol, RansEncoder,
    // RansEncoderMulti,
    // };
    //
    // const SCALE_BITS: u32 = 11;
    //
    // Encode two symbols
    // let mut encoder = ByteRansEncoder::new(1024);
    // let symbol1 = ByteRansEncSymbol::new(0, 2, SCALE_BITS);
    // let symbol2 = ByteRansEncSymbol::new(2, 2, SCALE_BITS);
    //
    // encoder.put(&symbol1);
    // encoder.put(&symbol2);
    // encoder.flush();
    //
    // let mut data = encoder.data().to_owned();
    //
    // println!("Data: {:?}", data);
    //
    // Decode the encoded data
    // let mut decoder = ByteRansDecoder::new(data);
    // let symbol1 = ByteRansDecSymbol::new(0, 2);
    // let symbol2 = ByteRansDecSymbol::new(2, 2);
    //
    // Please note that the data is being decoded in reverse
    // assert_eq!(decoder.get(SCALE_BITS), 2); // Decoder returns
    // cumulative frequency decoder.advance(&symbol2, SCALE_BITS);
    // assert_eq!(decoder.get(SCALE_BITS), 0);
    // decoder.advance(&symbol1, SCALE_BITS);
    //
    // Let's try a toy example with FASTQ
    //
    // let toy = b"AACTGAAATT";
    //
    // Get frequencies
    // let mut counts = [0; 256];
    // for &x in toy.iter() {
    // counts[x as usize] += 1;
    // }
    //
    // Get cumulative frequencies
    // let mut cumul = [0; 256];
    // let mut total = 0;
    // for i in 0..256 {
    // cumul[i] = total;
    // total += counts[i];
    // }
    //
    // Normalize - because you can't have 0 probability
    // let mut scale = 1;
    // for i in 0..256 {
    // if counts[i] != 0 {
    // cumul[i] = scale;
    // scale += counts[i];
    // }
    // }
    //
    // Create map
    // let mut map: HashMap<u8, (ByteRansEncSymbol, u32, u32)> =
    // HashMap::new();
    //
    // for i in toy {
    // let symbol = ByteRansEncSymbol::new(cumul[*i as usize], counts[*i
    // as usize], SCALE_BITS); map.insert(*i, (symbol, cumul[*i as
    // usize], counts[*i as usize])); }
    //
    // Map for conversion backwards
    // let mut map2: HashMap<u32, u8> = HashMap::new();
    // for i in toy {
    // for x in cumul[*i as usize]..(cumul[*i as usize] + counts[*i as
    // usize]) { If not already in map
    // if !map2.contains_key(&x) {
    // map2.insert(x, *i);
    // }
    // }
    // }
    //
    //
    // Real data
    //
    // let mut fastq = needletail::parse_fastx_file(
    // "/mnt/data/data/sfasta_testing/reads.fastq",
    // )
    // .unwrap();
    // let mut scores = Vec::new();
    // let mut sequences = Vec::new();
    // while let Some(record) = fastq.next() {
    // let record = record.unwrap();
    // scores.extend_from_slice(&record.qual().unwrap());
    // sequences.extend_from_slice(&record.seq());
    // if scores.len() >= 64 * 1024 {
    // break;
    // }
    // }
    //
    // Operate on blocks of 1024
    // let block_size = 1024;
    // let scores = scores[..block_size].to_vec();
    // let scores = sequences[..block_size].to_vec();
    //
    //
    // Get frequencies
    // let mut counts = [0; 256];
    // for &x in scores.iter() {
    // counts[x as usize] += 1;
    // }
    //
    // Get cumulative frequencies
    // let mut cumul = [0; 256];
    // let mut total = 0;
    // for i in 0..256 {
    // cumul[i] = total;
    // total += counts[i];
    // }
    //
    // Normalize - because you can't have 0 probability
    // let mut scale = 1;
    // for i in 0..256 {
    // if counts[i] != 0 {
    // cumul[i] = scale;
    // scale += counts[i];
    // }
    // }
    //
    // Create map
    // let mut map: HashMap<u8, (ByteRansEncSymbol, u32, u32)> =
    // HashMap::new();
    //
    // for i in scores.iter() {
    // let symbol = ByteRansEncSymbol::new(cumul[*i as usize], counts[*i
    // as usize], SCALE_BITS); map.insert(*i, (symbol, cumul[*i as
    // usize], counts[*i as usize])); }
    //
    // Map for conversion backwards
    // let mut map2: HashMap<u32, u8> = HashMap::new();
    // for i in scores.iter() {
    // for x in cumul[*i as usize]..(cumul[*i as usize] + counts[*i as
    // usize]) { If not already in map
    // if !map2.contains_key(&x) {
    // map2.insert(x, *i);
    // }
    // }
    // }
    //
    // Now encode
    // let mut encoder = ByteRansEncoder::new(scores.len());
    // for i in scores.iter() {
    // let (symbol, _, _) = &map[&i];
    // encoder.put(symbol);
    // }
    //
    // encoder.flush();
    //
    // let data = encoder.data().to_owned();
    // println!("Data: {:?}", data);
    // println!("Original data size: {}", scores.len());
    // println!("Encoded data size: {}", data.len());
    // println!("Compression Ratio: {:.2}", data.len() as f64 /
    // scores.len() as f64);
    //
    // Now try with the module
    // use rans_impl::*;
    //
    // let freqs = calculate_frequencies(&scores);
    // let mut cumul_freqs = to_cumulative_frequencies(&freqs);
    // normalize_frequencies(&mut cumul_freqs);
    //
    // let (scale_bits, symbols) =
    // generate_encoding_symbols(&cumul_freqs); let data =
    // encode(&scores, &cumul_freqs);
    //
    // println!("Data: {:?}", data);
    // println!("Original data size: {}", scores.len());
    // println!("Encoded data size: {}", data.len());
    // println!("Compression Ratio: {:.2}", data.len() as f64 /
    // scores.len() as f64);
    //
    //
    //
    //
    //
    //
    //
    //
    // panic!();
    // }
    //
    //
    // pub fn test_minimum_redundancy()
    // {
    // let mut compression_config = CompressionConfig::new();
    // compression_config.compression_type =
    // CompressionType::MINIMUMREDUNDANCY;
    //
    // let mut fastq = needletail::parse_fastx_file(
    // "/mnt/data/data/sfasta_testing/reads.fastq",
    // )
    // .unwrap();
    // let mut scores = Vec::new();
    // let mut sequences = Vec::new();
    // while let Some(record) = fastq.next() {
    // let record = record.unwrap();
    // scores.extend_from_slice(&record.qual().unwrap());
    // sequences.extend_from_slice(&record.seq());
    // if scores.len() >= 64 * 1024 {
    // break;
    // }
    // }
    //
    // println!("Scores Length: {}", scores.len());
    //
    // let data = scores;
    //
    // let huffman_compressed =
    // compression_config.compress(&data).unwrap(); println!(
    // "Orig Length: {} / Compressed Length: {}",
    // data.len(),
    // huffman_compressed.len()
    // );
    // println!(
    // "Ratio: {:.2}",
    // huffman_compressed.len() as f64 / data.len() as f64
    // );
    //
    // Zstd compress it
    // let compression_config = CompressionConfig::new();
    // let compression_config =
    // compression_config.with_compression_type(CompressionType::ZSTD);
    // let compression_config =
    // compression_config.with_compression_level(7); let compressed =
    // compression_config.compress(&data).unwrap();
    //
    // println!(
    // "Orig Length: {} / Zstd Length: {}",
    // data.len(),
    // compressed.len()
    // );
    // println!("Ratio: {:.2}", compressed.len() as f64 / data.len() as
    // f64);
    //
    // Compare against the huffman compressed + zstd compressed
    // let compressed =
    // compression_config.compress(&huffman_compressed).unwrap();
    // println!(
    // "Data Length: {} / Huffman Length: {} / Huffman+ZSTD Length: {}",
    // data.len(),
    // huffman_compressed.len(),
    // compressed.len()
    // );
    // println!("Ratio: {:.2}", compressed.len() as f64 / data.len() as
    // f64);
    //
    // println!("===============================Sequences============================");
    //
    // Repeat for sequences
    // let data = sequences;
    // let mut compression_config = CompressionConfig::new();
    // compression_config.compression_type =
    // CompressionType::MINIMUMREDUNDANCY;
    // let huffman_compressed =
    // compression_config.compress(&data).unwrap(); println!(
    // "Orig Length: {} / Compressed Length: {}",
    // data.len(),
    // huffman_compressed.len()
    // );
    // println!(
    // "Ratio: {:.2}",
    // huffman_compressed.len() as f64 / data.len() as f64
    // );
    //
    // Zstd compress it
    // let compression_config = CompressionConfig::new();
    // let compression_config =
    // compression_config.with_compression_type(CompressionType::ZSTD);
    // let compression_config =
    // compression_config.with_compression_level(7); let compressed =
    // compression_config.compress(&data).unwrap();
    //
    // println!(
    // "Orig Length: {} / Zstd Length: {}",
    // data.len(),
    // compressed.len()
    // );
    // println!("Ratio: {:.2}", compressed.len() as f64 / data.len() as
    // f64);
    //
    // Compare against the huffman compressed + zstd compressed
    // let compressed =
    // compression_config.compress(&huffman_compressed).unwrap();
    // println!(
    // "Data Length: {} / Huffman Length: {} / Huffman+ZSTD Length: {}",
    // data.len(),
    // huffman_compressed.len(),
    // compressed.len()
    // );
    // println!("Ratio: {:.2}", compressed.len() as f64 / data.len() as
    // f64);
    //
    // panic!();
    // }
    //

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

    // #[test]
    // pub fn test_fqzcomp() {
    // let scores =
    // b"-3331-,,,.22114568744.++++555:>?@AB<<AGHEA49?@AA?::(&&''(''
    // ((*---432+))'(-(*+*+,,,8776..-30/0112556677:2,,,++/58832334:<<;
    // 8::<=87998;>DCDA?@A><:66////.23327842:3**,,,,,))*('''(*./+((*
    // 14449200()))45-)))*6444**)))()((%$$$$%&&*',532222566.,---11137;
    // ;<>6-*))558::0.-+,0/+,::820..)&&&'68899<<<>>:EFH@DG=<886ABFD@9/
    // //./7>>9?C;C?BAC@A@<975+*****/000//111A@?:730-.??:(12:8533.//
    // --,-03:9:A:9C1.-,*+,069<?=<.--:8B83:?>AEES>89757:6;;<<<8@
    // 106545789@@FHGA?D<?>@IDDGDA<B@@CCCFDIADE=:;;40012@A;4><:
    // 865966656812/5/.-2413+*++.:<;88;><88>=4173688?@878==;>FIFDB?=:
    // 99:=7//CB86BA=DDIEAEDBGE?<==7950-EBC??42/,/=9=52GDB8D@A<<FB<8>:
    // 9>G:=B><SLC<:977532./+,-*777/./,*//6<:66FA0//..?ED=::<3-'%%&('
    // ((,3449::++5<DDHIACD<EA@GCB@744-+-63/.-/65:D8:II<837:66;>.BEE?
    // 6:9:6?EB><9<;8ACA;;:==;887646@KKI>:640/6@,))),844997789=8?7/*,:
    // C=8<B=998.-.2HAEJLSMSSI>JFAM6433022102>CA?>>A21/,3/,,0.,++-0./
    // 0/112987449:7AH?>4568C?6:<;<F3588;321665346801.-./01-,,,+,6>?;
    // 61176555443433229:7)(),,-00/.,026889999A=DDDIGJES@9:7=<69:
    // 886HACA:/+)AHKEC:4-)*-*)),,,/0.2335@;8<>:IBCH>;98:=49?D?E77677;
    // <89:8GSK6655GDFD874775FFE<1.-656421111644345>:;:8>BA@@
    // <GDSFI=83331112:<75438<>:2..)-)(((+5443342..2210/,/
    // 24554264677ABB=43334??.****--20"; let compressed =
    // CompressionConfig::new().
    // with_compression_type(CompressionType::FQZCOMP).
    // compress(scores).unwrap();

    // }
}
