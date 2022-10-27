use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::Arc;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam::queue::ArrayQueue;
use crossbeam::sync::{Parker, Unparker};
use crossbeam::utils::Backoff;
use rand::prelude::*;

use crate::datatypes::*;
use crate::CompressionType;

#[derive(Clone)]
pub struct CompressionStreamBufferConfig {
    pub block_size: u32,
    pub compression_type: CompressionType,
    pub compression_level: i8,
    pub num_threads: u16,
    pub compression_dict: Option<Vec<u8>>, // Only impl for Zstd at this time
}

impl Default for CompressionStreamBufferConfig {
    fn default() -> Self {
        Self {
            block_size: 1024 * 1024,
            compression_type: CompressionType::ZSTD,
            compression_level: 3,
            num_threads: 1,
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
        num_threads: u16,
    ) -> Self {
        Self {
            block_size,
            compression_type,
            compression_level,
            num_threads,
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

    pub const fn with_threads(mut self, num_threads: u16) -> Self {
        self.num_threads = num_threads;
        self
    }

    pub fn with_compression_dict(mut self, compression_dict: Vec<u8>) -> Self {
        self.compression_dict = Some(compression_dict);
        self
    }
}

pub struct CompressionStreamBuffer {
    block_size: u32,
    cur_block_id: u32,
    buffer: Vec<u8>,
    threads: u16,
    initialized: bool,
    workers: Vec<JoinHandle<()>>,
    workers_unparkers: Vec<Unparker>,
    sort_worker: Option<JoinHandle<()>>,
    sort_queue: Arc<ArrayQueue<(u32, SequenceBlockCompressed)>>,
    output_queue: Arc<ArrayQueue<(u32, SequenceBlockCompressed)>>,
    compress_queue: Arc<ArrayQueue<(u32, SequenceBlock)>>,
    shutdown: Arc<AtomicBool>,
    total_entries: Arc<AtomicUsize>,
    sorted_entries: Arc<AtomicUsize>,
    finalized: bool,
    compression_type: CompressionType,
    compression_level: i8,
    compression_dict: Option<Vec<u8>>,
    emit_block_spins: usize,
}

impl Default for CompressionStreamBuffer {
    fn default() -> Self {
        CompressionStreamBuffer {
            block_size: 2 * 1024 * 1024,
            cur_block_id: 0,
            buffer: Vec::with_capacity(2 * 1024 * 1024),
            threads: 1,
            initialized: false,
            workers: Vec::<JoinHandle<()>>::new(),
            workers_unparkers: Vec::<Unparker>::new(),
            sort_worker: None,
            sort_queue: Arc::new(ArrayQueue::new(16)),
            compress_queue: Arc::new(ArrayQueue::new(16)),
            output_queue: Arc::new(ArrayQueue::new(16)),
            shutdown: Arc::new(AtomicBool::new(false)),
            finalized: false,
            total_entries: Arc::new(AtomicUsize::new(0)),
            sorted_entries: Arc::new(AtomicUsize::new(0)),
            compression_type: CompressionType::ZSTD,
            compression_level: default_compression_level(CompressionType::ZSTD),
            emit_block_spins: 0,
            compression_dict: None,
        }
    }
}

impl Drop for CompressionStreamBuffer {
    fn drop(&mut self) {
        self.wakeup();

        assert!(
            self.buffer.is_empty(),
            "SequenceBuffer was not empty. Finalize the buffer to emit the final block."
        );
    }
}

#[allow(dead_code)]
impl CompressionStreamBuffer {
    pub fn from_config(config: CompressionStreamBufferConfig) -> Self {
        let mut buffer = Self::default();
        buffer.block_size = config.block_size;
        buffer.threads = config.num_threads;
        buffer.compression_type = config.compression_type;
        buffer.compression_level = config.compression_level;
        buffer.compression_dict = config.compression_dict;
        buffer
    }

    #[inline(always)]
    pub fn wakeup(&self) {
        if let Some(sort_worker) = &self.sort_worker {
            sort_worker.thread().unpark();
        }

        for i in self.workers_unparkers.iter() {
            i.unpark()
        }
    }

    pub fn initialize(&mut self) {
        let mut rng = rand::thread_rng();

        for _ in 0..self.threads {
            let shutdown_copy = Arc::clone(&self.shutdown);
            let cq = Arc::clone(&self.compress_queue);
            let wq = Arc::clone(&self.sort_queue);

            let ct = self.compression_type;
            let cl = self.compression_level;
            let cd = self.compression_dict.clone();

            let sleep = std::time::Duration::from_micros(rng.gen_range(1..100));

            let parker = Parker::new();
            let parker_thread = parker.unparker().clone();
            self.workers_unparkers.push(parker_thread);

            let handle = thread::spawn(move || {
                std::thread::sleep(sleep);
                _compression_worker_thread(cq, wq, shutdown_copy, ct, cl, cd, parker)
            });
            self.workers.push(handle);
        }

        {
            let shutdown_copy = Arc::clone(&self.shutdown);
            let wq = Arc::clone(&self.sort_queue);
            let we = Arc::clone(&self.sorted_entries);
            let oq = Arc::clone(&self.output_queue);
            let parker = Parker::new();
            let parker_thread = parker.unparker().clone();
            let handle = thread::spawn(move || _sorter_worker_thread(wq, oq, shutdown_copy, we));

            self.sort_worker = Some(handle);
        }

        self.initialized = true;
    }

    pub fn finalize(&mut self) -> Result<(), &'static str> {
        // Emit the final block...
        self.emit_block();
        self.finalized = true;
        self.wakeup();

        let backoff = Backoff::new();

        log::debug!("Waiting for compression workers to finish...");

        while self.sorted_entries.load(Ordering::Relaxed)
            < self.total_entries.load(Ordering::Relaxed)
        {
            self.wakeup();
            backoff.snooze();
        }

        self.shutdown.store(true, Ordering::Relaxed);

        log::debug!("Joining workers");

        self.wakeup();

        for i in self.workers.drain(..) {
            i.join().unwrap();
        }

        let writer_handle = std::mem::replace(&mut self.sort_worker, None);

        writer_handle.unwrap().join().unwrap();
        log::debug!("CompressionStreamBuffer finalized");
        log::debug!("Emit block spins: {}", self.emit_block_spins);
        Ok(())
    }

    /// Returns the Locations for the sequence, and the locations for the masking (if any)
    pub fn add_sequence(&mut self, x: &mut [u8]) -> Result<Vec<Loc>, &'static str> {
        assert!(self.block_size > 0);
        assert!(!self.finalized, "SeqBuffer has been finalized.");

        let block_size = self.block_size as usize;

        let mut locs = Vec::with_capacity(x.len() / self.block_size as usize);

        // Remove whitespace
        let mut seq = x;
        seq.make_ascii_uppercase();

        while !seq.is_empty() {
            let len = self.len();

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

            let loc = if len as u32 == 0 && len as u32 + end as u32 == self.block_size {
                Loc::EntireBlock(self.cur_block_id)
            } else if len == 0 {
                Loc::FromStart(self.cur_block_id, len as u32 + end as u32)
            } else if len + end == self.block_size as usize {
                Loc::ToEnd(self.cur_block_id, len as u32)
            } else {
                Loc::Loc(self.cur_block_id, len as u32, len as u32 + end as u32)
            };

            locs.push(loc);

            if self.len() >= block_size {
                self.emit_block();
            }

            seq = &mut seq[end..];
        }

        self.wakeup();

        Ok(locs)
    }

    pub fn emit_block(&mut self) {
        let backoff = Backoff::new();
        let mut seq = Vec::with_capacity(self.block_size as usize);
        std::mem::swap(&mut self.buffer, &mut seq);

        let x = SequenceBlock { seq };
        self.wakeup();

        if x.is_empty() {
            return;
        }

        self.total_entries.fetch_add(1, Ordering::SeqCst);

        let mut entry = (self.cur_block_id, x);
        while let Err(x) = self.compress_queue.push(entry) {
            self.emit_block_spins = self.emit_block_spins.saturating_add(1);
            backoff.snooze();
            entry = x;
        }
        self.cur_block_id += 1;

        self.wakeup();
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
        self.compression_type = compression_type;
        self
    }

    pub fn get_output_queue(&self) -> Arc<ArrayQueue<(u32, SequenceBlockCompressed)>> {
        Arc::clone(&self.output_queue)
    }

    pub fn get_shutdown_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.shutdown)
    }
}

fn _compression_worker_thread(
    compress_queue: Arc<ArrayQueue<(u32, SequenceBlock)>>,
    sort_queue: Arc<ArrayQueue<(u32, SequenceBlockCompressed)>>,
    shutdown: Arc<AtomicBool>,
    compression_type: CompressionType,
    compression_level: i8,
    compression_dict: Option<Vec<u8>>,
    parker: Parker,
) {
    let mut result;
    let backoff = Backoff::new();
    let mut compression_worker_spins: usize = 0;
    let mut work_units: usize = 0;

    let mut zstd_compressor = if compression_type == CompressionType::ZSTD {
        Some(zstd_encoder(compression_level as i32, compression_dict))
    } else {
        None
    };

    let mut rng = rand::thread_rng();

    loop {
        result = compress_queue.pop();

        match result {
            None => {
                if shutdown.load(Ordering::Relaxed) {
                    log::debug!(
                        "Compression worker thread spins: {}",
                        compression_worker_spins
                    );
                    log::debug!("Compression worker thread work units: {}", work_units);
                    return;
                } else {
                    backoff.spin();
                    compression_worker_spins = compression_worker_spins.saturating_add(1);
                    if backoff.is_completed() {
                        parker.park_timeout(Duration::from_millis(rng.gen_range(50..150)));
                        backoff.reset();
                    }
                }
            }

            Some((block_id, sb)) => {
                work_units += 1;
                let sbc = sb.compress(
                    compression_type,
                    compression_level,
                    zstd_compressor.as_mut(),
                );

                let mut entry = (block_id, sbc);
                while let Err(x) = sort_queue.push(entry) {
                    log::debug!("Compression Worker Sort Queue Full");
                    entry = x;
                }
            }
        }
    }
}

fn _sorter_worker_thread(
    sort_queue: Arc<ArrayQueue<(u32, SequenceBlockCompressed)>>,
    output_queue: Arc<ArrayQueue<(u32, SequenceBlockCompressed)>>,
    shutdown: Arc<AtomicBool>,
    written_entries: Arc<AtomicUsize>,
) {
    let mut expected_block: u32 = 0;
    let mut queue: HashMap<u32, SequenceBlockCompressed> = HashMap::with_capacity(256);
    let mut result;
    let backoff = Backoff::new();

    let mut sorter_worker_empty_spins: usize = 0;
    let mut sorter_worker_have_blocks_spins: usize = 0;
    let mut sorter_worker_output_spins: usize = 0;

    loop {
        backoff.reset();
        while !queue.is_empty() && queue.contains_key(&expected_block) {
            let sbc = queue.remove(&expected_block).unwrap();

            let mut entry = (expected_block, sbc);
            while let Err(x) = output_queue.push(entry) {
                sorter_worker_output_spins = sorter_worker_output_spins.saturating_add(1);
                backoff.snooze();
                entry = x;
            }

            expected_block += 1;
            written_entries.fetch_add(1, Ordering::SeqCst);
        }

        while sort_queue.is_empty() {
            if shutdown.load(Ordering::Relaxed) {
                log::debug!(
                    "Sorter worker empty spins: {}, have blocks spins: {}, output spins: {}",
                    sorter_worker_empty_spins,
                    sorter_worker_have_blocks_spins,
                    sorter_worker_output_spins
                );
                return;
            }

            backoff.snooze();

            if queue.is_empty() {
                sorter_worker_empty_spins = sorter_worker_empty_spins.saturating_add(1);
            } else {
                sorter_worker_have_blocks_spins = sorter_worker_have_blocks_spins.saturating_add(1);
            }

            if backoff.is_completed() {
                backoff.reset();
            }
        }

        result = sort_queue.pop();

        match result {
            None => {
                if shutdown.load(Ordering::Relaxed) {
                    log::debug!(
                        "Sorter worker empty spins: {}, have blocks spins: {}, output spins: {}",
                        sorter_worker_empty_spins,
                        sorter_worker_have_blocks_spins,
                        sorter_worker_output_spins
                    );
                    return;
                }
            }
            Some((block_id, sbc)) => {
                if block_id == expected_block {
                    let mut entry = (expected_block, sbc);
                    while let Err(x) = output_queue.push(entry) {
                        sorter_worker_output_spins = sorter_worker_output_spins.saturating_add(1);
                        backoff.snooze();
                        entry = x;
                    }
                    expected_block += 1;
                    written_entries.fetch_add(1, Ordering::SeqCst);
                } else {
                    queue.insert(block_id, sbc);
                }
            }
        }
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

        sb.wakeup();
        // println!("Done adding seqs...");
        sb.finalize().expect("Unable to finalize SeqBuffer");

        // println!("Finalized...");

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
