use std::sync::atomic::Ordering;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::Arc;

use crossbeam::queue::ArrayQueue;
use crossbeam::utils::Backoff;

use crate::compression::{CompressionConfig, CompressionWork, CompressionWorkerOrder, Worker};
use crate::datatypes::*;
use crate::CompressionType;

#[derive(Clone)]
pub struct CompressionStreamBufferConfig {
    pub block_size: u32,
    pub compression_type: CompressionType,
    pub compression_level: i8,
    pub compression_dict: Option<Arc<Vec<u8>>>, // Only impl for Zstd at this time
}

impl Default for CompressionStreamBufferConfig {
    fn default() -> Self {
        Self {
            block_size: 512 * 1024,
            compression_type: CompressionType::ZSTD,
            compression_level: 3,
            compression_dict: None,
        }
    }
}

#[allow(dead_code)]
impl CompressionStreamBufferConfig {
    pub const fn new(
        block_size: u32,
        compression_type: CompressionType,
        compression_level: i8,
    ) -> Self {
        Self {
            block_size,
            compression_type,
            compression_level,
            compression_dict: None,
        }
    }

    pub const fn with_block_size(mut self, block_size: u32) -> Self {
        self.block_size = block_size;
        self
    }

    pub const fn with_compression_type(mut self, compression_type: CompressionType) -> Self {
        self.compression_type = compression_type;
        self.compression_level = default_compression_level(compression_type);
        self
    }

    pub const fn with_compression_level(mut self, compression_level: i8) -> Self {
        self.compression_level = compression_level;
        self
    }

    pub fn with_compression_dict(mut self, compression_dict: Vec<u8>) -> Self {
        self.compression_dict = Some(Arc::new(compression_dict));
        self
    }
}

pub struct CompressionStreamBuffer {
    block_size: u32,
    cur_block_id: u32,
    buffer: Vec<u8>,
    threads: u16,
    initialized: bool,
    output_queue: Arc<ArrayQueue<(u32, SequenceBlockCompressed)>>,
    shutdown: Arc<AtomicBool>,
    total_entries: Arc<AtomicUsize>,
    sorted_entries: Arc<AtomicUsize>,
    finalized: bool,
    emit_block_spins: usize,
    compression_worker: Option<Arc<Worker>>,
    compression_config: Option<CompressionConfig>,
}

impl Default for CompressionStreamBuffer {
    fn default() -> Self {
        CompressionStreamBuffer {
            block_size: 2 * 1024 * 1024,
            cur_block_id: 0,
            buffer: Vec::with_capacity(2 * 1024 * 1024),
            threads: 1,
            initialized: false,
            output_queue: Arc::new(ArrayQueue::new(16)),
            shutdown: Arc::new(AtomicBool::new(false)),
            finalized: false,
            total_entries: Arc::new(AtomicUsize::new(0)),
            sorted_entries: Arc::new(AtomicUsize::new(0)),
            emit_block_spins: 0,
            compression_worker: None,
            compression_config: None,
        }
    }
}

impl Drop for CompressionStreamBuffer {
    fn drop(&mut self) {
        assert!(
            self.buffer.is_empty(),
            "SequenceBuffer was not empty. Finalize the buffer to emit the final block."
        );
    }
}

impl CompressionStreamBuffer {
    pub fn from_config(config: CompressionStreamBufferConfig) -> Self {
        let mut buffer = Self::default();

        let compression_config = Some(
            CompressionConfig::new()
                .with_compression_type(config.compression_type)
                .with_compression_level(config.compression_level)
                .with_compression_dict(config.compression_dict),
        );

        buffer.block_size = config.block_size;

        buffer
    }

    pub fn initialize(&mut self) {
        self.initialized = true;
    }

    pub fn finalize(&mut self) -> Result<(), &'static str> {
        // Emit the final block...
        self.emit_block();
        self.finalized = true;

        let backoff = Backoff::new();

        log::info!("Waiting for compression workers to finish...");

        while self.sorted_entries.load(Ordering::Relaxed)
            < self.total_entries.load(Ordering::Relaxed)
        {
            backoff.snooze();
        }

        self.shutdown.store(true, Ordering::Relaxed);

        log::info!("Emit block spins: {}", self.emit_block_spins);
        Ok(())
    }

    /// Returns the Locations for the sequence, and the locations for the masking (if any)
    pub fn add_sequence(&mut self, x: &mut [u8]) -> Result<Vec<Loc>, &'static str> {
        assert!(self.block_size > 0);
        assert!(!self.finalized, "SeqBuffer has been finalized.");

        let block_size = self.block_size as usize;

        let mut locs = Vec::with_capacity((x.len() / self.block_size as usize) + 8);

        // Remove whitespace
        let mut seq = x;
        seq.make_ascii_uppercase();

        while !seq.is_empty() {
            let len = self.len(); // Length of current block

            let mut end = seq.len();

            // Sequence will run past the end of the block...
            if len + seq.len() >= block_size {
                end = block_size - len;
            }

            //self.buffer.extend_from_slice(seq[0..end].trim_ascii());
            // Until trim_ascii is stabilized...

            let mut effective_end = end;
            while effective_end > 0 && seq[effective_end - 1].is_ascii_whitespace() {
                effective_end -= 1;
            }

            let mut effective_start = 0;
            while effective_start < effective_end && seq[effective_start].is_ascii_whitespace() {
                effective_start += 1;
            }

            self.buffer
                .extend_from_slice(&seq[effective_start..effective_end]);

            let loc = Loc {
                block: self.cur_block_id,
                start: len as u32,
                len: end as u32,
            };

            locs.push(loc);

            if self.len() >= block_size {
                self.emit_block();
            }

            seq = &mut seq[end..];
        }

        Ok(locs)
    }

    pub fn emit_block(&mut self) {
        let backoff = Backoff::new();
        let mut seq = Vec::with_capacity(self.block_size as usize);
        std::mem::swap(&mut self.buffer, &mut seq);

        let x = SequenceBlock { seq };

        if x.is_empty() {
            return;
        }

        self.total_entries.fetch_add(1, Ordering::SeqCst);

        let mut entry = CompressionWorkerOrder::Compress(seq);
        self.compression_worker.as_ref().unwrap().submit(entry);
        self.cur_block_id += 1;
    }

    // Convenience functions

    fn len(&mut self) -> usize {
        self.buffer.len()
    }

    fn _check_initialized(&mut self) {
        assert!(!self.initialized, "SeqBuffer already initialized...")
    }

    pub fn with_block_size(mut self, block_size: u32) -> Self {
        self._check_initialized();
        self.block_size = block_size;
        self.buffer = Vec::with_capacity(block_size as usize);
        self
    }

    pub fn with_threads(mut self, threads: u16) -> Self {
        self._check_initialized();
        self.threads = threads;
        self
    }

    pub fn with_compression_type(mut self, compression_type: CompressionType) -> Self {
        self._check_initialized();
        self
    }

    pub fn get_output_queue(&self) -> Arc<ArrayQueue<(u32, SequenceBlockCompressed)>> {
        Arc::clone(&self.output_queue)
    }

    pub fn get_shutdown_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.shutdown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn test_add_sequence() {
        let mut myseq = b"ACTGGGGGGGG".to_vec();

        let test_block_size = 512 * 1024;

        let mut sb = CompressionStreamBuffer::default()
            .with_block_size(test_block_size)
            .with_threads(2);

        #[cfg(miri)]
        let mut sb = sb.with_compression_type(CompressionType::NONE);

        sb.initialize();

        let oq = sb.get_output_queue();

        let mut locs = Vec::new();

        let loc = sb.add_sequence(&mut myseq[..]).unwrap();
        locs.extend(loc);

        let mut largeseq = Vec::new();
        while largeseq.len() <= 4 * 1024 * 1024 {
            largeseq.extend_from_slice(&myseq[..]);
        }

        let loc = sb.add_sequence(&mut largeseq).unwrap();
        locs.extend(loc);

        sb.finalize().expect("Unable to finalize SeqBuffer");

        // This is actually a test, just making sure it does not panic here
        drop(sb);

        assert!(oq.len() == 9);
        let g = oq.pop().unwrap();
        let mut zstd_decompressor = zstd::bulk::Decompressor::new().unwrap();
        zstd_decompressor.include_magicbytes(false).unwrap();
        let g = g.1.decompress(
            CompressionType::ZSTD,
            8 * 1024 * 1024,
            Some(zstd_decompressor).as_mut(),
        );
        println!("{:#?}", g.len());
        assert!(g.len() == 524288);
        let seq = std::str::from_utf8(&g.seq[0..100]).unwrap();
        //println!("{:#?}", seq);
        assert!(seq == "ACTGGGGGGGGACTGGGGGGGGACTGGGGGGGGACTGGGGGGGGACTGGGGGGGGACTGGGGGGGGACTGGGGGGGGACTGGGGGGGGACTGGGGGGGGA");

        /* assert!(locs.len() == 10);
        println!("Blocks: {}", block_idx.len());
        assert!(block_idx.len() == 9); */

        /*

        let mut buf_out: &mut Box<Cursor<Vec<u8>>> = Any::downcast_mut(&mut buf_out).unwrap();
        let mut buf_out = buf_out.to_owned();

        buf_out.seek(SeekFrom::Start(0)).unwrap();

        let mut buf_out = BufReader::new(buf_out);

        let decoded: SequenceBlockCompressed = bincode::deserialize_from(&mut buf_out).unwrap();
        assert!(decoded.decompress().len() == test_block_size as usize);

        let decoded: SequenceBlockCompressed = bincode::deserialize_from(&mut buf_out).unwrap();
        assert!(decoded.decompress().len() == test_block_size as usize);

        let decoded: SequenceBlockCompressed = bincode::deserialize_from(&mut buf_out).unwrap();
        let decoded = decoded.decompress();
        let seq = std::str::from_utf8(&decoded.seq[0..100]).unwrap();
        assert!(seq == "CTGGGGGGGGACTGGGGGGGGACTGGGGGGGGACTGGGGGGGGACTGGGGGGGGACTGGGGGGGGACTGGGGGGGGACTGGGGGGGGACTGGGGGGGGAC");
        */
    }

    #[test]
    #[should_panic(
        expected = "SequenceBuffer was not empty. Finalize the buffer to emit the final block."
    )]
    pub fn test_lingering_sequence_in_seqbuf() {
        let mut myseq = b"ACTGGGGGGGG".to_vec();

        let test_block_size = 512 * 1024;

        let mut sb = CompressionStreamBuffer::default().with_block_size(test_block_size);

        sb.add_sequence(&mut myseq[..])
            .expect("Error adding sequence");
    }
}
