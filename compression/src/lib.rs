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

pub mod dict;

pub use dict::*;

pub const MAX_DECOMPRESS_SIZE: usize = 1024 * 1024 * 1024; // 1GB

use std::{cell::RefCell, rc::Rc};

thread_local! {
    static ZSTD_COMPRESSOR: RefCell<zstd::bulk::Compressor<'static>> = RefCell::new(zstd_encoder(3, &None));
    static ZSTD_DECOMPRESSOR: RefCell<zstd::bulk::Decompressor<'static>> = RefCell::new(zstd_decompressor(None));

    // todo, all the others
}

use crossbeam::{queue::ArrayQueue, utils::Backoff};

#[cfg(not(target_arch = "wasm32"))]
use xz::read::{XzDecoder, XzEncoder};

#[derive(Debug, Clone)]
pub struct OutputBlock
{
    pub data: Vec<u8>,
    pub location: Arc<AtomicU64>,
}

#[derive(Debug, Clone, bincode::Encode, bincode::Decode)]
pub struct CompressionConfig
{
    pub compression_type: CompressionType,
    pub compression_level: i8,
    pub compression_dict: Option<Arc<Vec<u8>>>,
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

    pub const fn with_compression_type(mut self, compression_type: CompressionType) -> Self
    {
        self.compression_type = compression_type;
        self
    }

    pub const fn with_compression_level(mut self, compression_level: i8) -> Self
    {
        self.compression_level = compression_level;
        self
    }

    pub fn with_compression_dict(mut self, compression_dict: Option<Arc<Vec<u8>>>) -> Self
    {
        self.compression_dict = compression_dict;
        self
    }

    pub fn compress(&self, bytes: &[u8]) -> Result<Vec<u8>, std::io::Error>
    {
        match self.compression_type {
            #[cfg(not(target_arch = "wasm32"))]
            CompressionType::ZSTD => {
                // Use thread local
                ZSTD_COMPRESSOR.with_borrow_mut(|zstd_compressor| zstd_compressor.compress(bytes))
            }
            #[cfg(target_arch = "wasm32")]
            CompressionType::ZSTD => {
                unimplemented!("ZSTD encoding is not supported on wasm32");
            }
            CompressionType::NONE => {
                log::debug!("Compress called for none, which involes copying bytes. Prefer not to use it!");
                Ok(bytes.to_vec())
            }
            _ => {
                // Todo: implement others. This isn't just used for fractaltrees...
                panic!("Unsupported compression type: {:?}", self.compression_type);
            }
        }
    }

    pub fn decompress(&self, bytes: &[u8]) -> Result<Vec<u8>, std::io::Error>
    {
        match self.compression_type {
            #[cfg(not(target_arch = "wasm32"))]
            CompressionType::ZSTD => ZSTD_DECOMPRESSOR
                .with_borrow_mut(|zstd_decompressor| zstd_decompressor.decompress(bytes, MAX_DECOMPRESS_SIZE)),
            #[cfg(target_arch = "wasm32")]
            CompressionType::ZSTD => {
                unimplemented!("ZSTD decoding is not supported on wasm32");
            }
            CompressionType::NONE => {
                log::debug!("Decompress called for none, which involes copying bytes. Prefer not to use it!");
                Ok(bytes.to_vec())
            }
            _ => {
                panic!("Unsupported compression type: {:?}", self.compression_type);
            }
        }
    }
}

#[derive(PartialEq, Eq, Debug, Clone, Copy, bincode::Encode, bincode::Decode)]
#[non_exhaustive]
pub enum CompressionType
{
    ZSTD,    // 1 should be default compression ratio
    LZ4,     // 9 should be default compression ratio
    SNAPPY,  // Not yet implemented -- IMPLEMENT
    GZIP,    // Please don't use this -- IMPLEMENT
    NAF,     // Not yet supported -- IMPLEMENT
    NONE,    // No Compression -- IMPLEMENT
    XZ,      // Implemented, 6 is default ratio
    BROTLI,  // Implemented, 6 is default
    NAFLike, // 1 should be default compression rate
    BZIP2,   // Unsupported
    LZMA,    // Unsupported
    RAR,     // Unsupported
}

impl Default for CompressionType
{
    fn default() -> CompressionType
    {
        CompressionType::ZSTD
    }
}

pub const fn default_compression_level(ct: CompressionType) -> i8
{
    match ct {
        CompressionType::ZSTD => 3,
        CompressionType::LZ4 => 9,
        CompressionType::XZ => 6,
        CompressionType::BROTLI => 9,
        CompressionType::NAFLike => 1,
        _ => 3,
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn zstd_encoder(compression_level: i32, dict: &Option<Arc<Vec<u8>>>) -> zstd::bulk::Compressor<'static>
{
    let mut encoder = if let Some(dict) = dict {
        zstd::bulk::Compressor::with_dictionary(compression_level, &dict).unwrap()
    } else {
        zstd::bulk::Compressor::new(compression_level).unwrap()
    };
    encoder.include_checksum(false).unwrap();
    encoder
        .include_magicbytes(false)
        .expect("Unable to set ZSTD MagicBytes");
    // encoder
    //.include_contentsize(false)
    //.expect("Unable to set ZSTD Content Size Flag");
    encoder
        .long_distance_matching(true)
        .expect("Unable to set long_distance_matching");
    // encoder.window_log(31);
    encoder
}

pub fn change_compression_level(compression_level: i32, dict: &Option<Arc<Vec<u8>>>)
{
    ZSTD_COMPRESSOR.with(|zstd_compressor| {
        let encoder = zstd_encoder(compression_level, dict);
        *zstd_compressor.borrow_mut() = encoder;
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub fn zstd_decompressor<'a>(dict: Option<&[u8]>) -> zstd::bulk::Decompressor<'a>
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

#[cfg(target_arch = "wasm32")]
pub fn zstd_decompressor<'a>(dict: Option<&[u8]>) -> zstd_dict::bulk::Decompressor<'a>
{
    unimplemented!("ZSTD decoding is not supported on wasm32");
}

#[cfg(target_arch = "wasm32")]
pub fn zstd_encoder(compression_level: i32) -> zstd_dict::bulk::Compressor<'static>
{
    unimplemented!("ZSTD encoding is not supported on wasm32");
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
    pub fn new(input: Vec<u8>, compression_config: Arc<CompressionConfig>) -> Self
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
    queue: Option<Arc<ArrayQueue<CompressorWork>>>,  // Input
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

    pub fn with_output_queue(mut self, writer: Arc<flume::Sender<OutputBlock>>) -> Self
    {
        self.writer = Some(writer);
        self
    }

    pub fn start(&mut self)
    {
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

    #[inline]
    pub fn submit(&self, work: CompressorWork)
    {
        let backoff = Backoff::new();

        let mut entry = work;
        while let Err(x) = self.queue.as_ref().unwrap().push(entry) {
            log::debug!("Compression Worker Buffer is Full: {}", self.queue.as_ref().unwrap().len());
            backoff.snooze();
            entry = x;
        }
    }

    #[inline]
    pub fn compress(&self, input: Vec<u8>, compression_config: Arc<CompressionConfig>) -> Arc<AtomicU64>
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
                let compressed = match work.compression_config.compression_type {
                    #[cfg(not(target_arch = "wasm32"))]
                    CompressionType::ZSTD => {
                        ZSTD_COMPRESSOR.with_borrow_mut(|zstd_compressor| {
                            zstd_compressor
                                .set_compression_level(work.compression_config.compression_level as i32)
                                .unwrap();
                            // todo dict
                            zstd_compressor.compress(work.input.as_slice()).unwrap()
                        })
                    }
                    #[cfg(target_arch = "wasm32")]
                    CompressionType::ZSTD => {
                        unimplemented!("ZSTD encoding is not supported on wasm32");
                    }
                    CompressionType::LZ4 => {
                        let mut output = Vec::with_capacity(work.input.len());
                        let mut lz4_compressor = lz4_flex::frame::FrameEncoder::new(&mut output);
                        lz4_compressor.write_all(work.input.as_slice()).unwrap();
                        lz4_compressor.finish().unwrap();
                        output
                    }
                    CompressionType::SNAPPY => {
                        let mut output = Vec::with_capacity(work.input.len());
                        let mut compressor = snap::write::FrameEncoder::new(&mut output);
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
                            flate2::Compression::new(work.compression_config.compression_level as u32),
                        );
                        compressor
                            .write_all(work.input.as_slice())
                            .expect("Unable to compress with GZIP");
                        compressor.finish().unwrap();
                        output
                    }
                    CompressionType::NONE => work.input,
                    #[cfg(not(target_arch = "wasm32"))]
                    CompressionType::XZ => {
                        let mut output = Vec::with_capacity(work.input.len());
                        let mut compressor =
                            XzEncoder::new(work.input.as_slice(), work.compression_config.compression_level as u32);
                        compressor.read_to_end(&mut output).expect("Unable to compress with XZ");
                        output
                    }
                    #[cfg(target_arch = "wasm32")]
                    CompressionType::XZ => {
                        panic!("XZ compression is not supported on wasm32");
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
                    log::debug!("Output queue is full");
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

#[cfg(test)]
mod tests
{
    use super::*;
    use flate2::Compression;
}
