use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Cursor, SeekFrom};
use std::sync::atomic::Ordering;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::Arc;
use std::thread;
use std::thread::{park, JoinHandle};

use crossbeam::queue::ArrayQueue;
use crossbeam::utils::Backoff;

use crate::sequence_block::*;
use crate::structs::{ReadAndSeek, WriteAndSeek};

pub struct SequenceBuffer {
    block_size: u32,
    cur_block_id: u32,
    buffer: Vec<u8>,
    threads: u16,
    initialized: bool,
    workers: Vec<JoinHandle<()>>,
    write_worker: Option<JoinHandle<()>>,
    write_queue: Arc<ArrayQueue<(u32, SequenceBlockCompressed)>>,
    output_queue: Arc<ArrayQueue<(u32, SequenceBlockCompressed)>>,
    compress_queue: Arc<ArrayQueue<(u32, SequenceBlock)>>,
    shutdown: Arc<AtomicBool>,
    total_entries: Arc<AtomicUsize>,
    written_entries: Arc<AtomicUsize>,
    finalized: bool,
}

impl Default for SequenceBuffer {
    fn default() -> Self {
        SequenceBuffer {
            block_size: 2 * 1024 * 1024,
            cur_block_id: 0,
            buffer: Vec::with_capacity(2 * 1024 * 1024),
            threads: 1,
            initialized: false,
            workers: Vec::<JoinHandle<()>>::new(),
            write_worker: None,
            write_queue: Arc::new(ArrayQueue::new(16)),
            compress_queue: Arc::new(ArrayQueue::new(64)),
            output_queue: Arc::new(ArrayQueue::new(16)),
            shutdown: Arc::new(AtomicBool::new(false)),
            finalized: false,
            total_entries: Arc::new(AtomicUsize::new(0)),
            written_entries: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Drop for SequenceBuffer {
    fn drop(&mut self) {
        assert!(
            self.buffer.len() == 0,
            "SequenceBuffer was not empty. Finalize the buffer to emit the final block."
        );
    }
}

impl SequenceBuffer {
    pub fn unpark(&mut self) {
        for j in &self.workers {
            j.thread().unpark();
        }

        self.write_worker.as_ref().unwrap().thread().unpark();
    }

    pub fn initialize(&mut self) {
        for i in 0..self.threads {
            let shutdown_copy = Arc::clone(&self.shutdown);
            let cq = Arc::clone(&self.compress_queue);
            let wq = Arc::clone(&self.write_queue);
            let handle = thread::spawn(move || _compression_worker_thread(cq, wq, shutdown_copy));

            self.workers.push(handle);
        }

        {
            let shutdown_copy = Arc::clone(&self.shutdown);
            let wq = Arc::clone(&self.write_queue);
            let we = Arc::clone(&self.written_entries);
            let oq = Arc::clone(&self.output_queue);
            let handle = thread::spawn(move || _writer_worker_thread(wq, oq, shutdown_copy, we));

            self.write_worker = Some(handle);
        }

        self.initialized = true;
    }

    pub fn finalize(&mut self) -> Result<(), &'static str> {
        // Emit the final block...
        self.emit_block();
        self.unpark();
        self.finalized = true;

        let backoff = Backoff::new();

        while self.written_entries.load(Ordering::Relaxed)
            < self.total_entries.load(Ordering::Relaxed)
        {
            self.unpark();
            backoff.snooze();
        }

        self.shutdown.store(true, Ordering::Relaxed);
        self.unpark();

        for i in self.workers.drain(..) {
            i.join().unwrap();
        }

        let writer_handle = std::mem::replace(&mut self.write_worker, None);

        writer_handle.unwrap().join().unwrap();
        Ok(())
    }

    pub fn add_sequence(&mut self, x: &[u8]) -> Result<Vec<(u32, (u64, u64))>, &'static str> {
        assert!(self.block_size > 0);
        assert!(self.finalized == false, "SeqBuffer has been finalized.");
        if !self.initialized {
            self.initialize();
        }

        let mut locs = Vec::new();

        let block_size = self.block_size as usize;

        let mut seq = &x[..];

        while seq.len() > 0 {
            let len = self.len();

            let mut end = seq.len();

            // Sequence will run past the end of the block...
            if len + seq.len() >= block_size {
                end = block_size - len;
            }

            self.buffer.extend_from_slice(&seq[0..end]);

            locs.push((self.cur_block_id, (len as u64, (len + end) as u64)));

            if self.len() >= block_size {
                self.emit_block();
            }

            seq = &seq[end..];
        }

        self.unpark();

        Ok(locs)
    }

    pub fn emit_block(&mut self) {
        let newbuf = Vec::with_capacity(self.block_size as usize);
        let seq = std::mem::replace(&mut self.buffer, newbuf);

        let x = SequenceBlock {
            seq,
            ..Default::default()
        };

        self.unpark();

        if x.len() == 0 {
            return;
        }

        self.total_entries.fetch_add(1, Ordering::SeqCst);

        let mut entry = (self.cur_block_id, x);
        while let Err(x) = self.compress_queue.push(entry) {
            entry = x;
            self.unpark();
        }
        self.cur_block_id += 1;
    }

    // Convenience functions

    fn len(&mut self) -> usize {
        self.buffer.len()
    }

    fn _check_initialized(&mut self) {
        assert!(
            self.initialized == false,
            "SeqBuffer already initialized..."
        )
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
        self.compress_queue = Arc::new(ArrayQueue::new(threads as usize * 16));
        self.write_queue = Arc::new(ArrayQueue::new(threads as usize * 4));
        self
    }

    pub fn output_queue(&self) -> Arc<ArrayQueue<(u32, SequenceBlockCompressed)>> {
        Arc::clone(&self.output_queue)
    }

    pub fn shutdown_notified(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.shutdown)
    }
}

fn _compression_worker_thread(
    compress_queue: Arc<ArrayQueue<(u32, SequenceBlock)>>,
    write_queue: Arc<ArrayQueue<(u32, SequenceBlockCompressed)>>,
    shutdown: Arc<AtomicBool>,
) {
    let mut result;
    let backoff = Backoff::new();
    loop {
        result = compress_queue.pop();

        match result {
            None => {
                backoff.snooze();
                if shutdown.load(Ordering::Relaxed) {
                    return;
                }
                park();
            }
            Some((block_id, sb)) => {
                let sbc = sb.compress();
                let mut entry = (block_id, sbc);
                while let Err(x) = write_queue.push(entry) {
                    entry = x;
                    println!("Queue Full!");
                    park(); // Queue is full, park the thread...
                }
            }
        }
    }
}

fn _writer_worker_thread(
    write_queue: Arc<ArrayQueue<(u32, SequenceBlockCompressed)>>,
    output_queue: Arc<ArrayQueue<(u32, SequenceBlockCompressed)>>,
    shutdown: Arc<AtomicBool>,
    written_entries: Arc<AtomicUsize>,
) {
    let mut expected_block: u32 = 0;
    let mut queue: HashMap<u32, SequenceBlockCompressed> = HashMap::new();
    let mut block_index: Vec<(u32, u64)> = Vec::new();
    let mut result;
    let backoff = Backoff::new();

    loop {
        while queue.contains_key(&expected_block) {
            let sbc = queue.remove(&expected_block).unwrap();

            let mut entry = (expected_block, sbc);
            while let Err(x) = output_queue.push(entry) {
                backoff.snooze();
                entry = x;
            }

            expected_block += 1;
            written_entries.fetch_add(1, Ordering::SeqCst);
        }

        result = write_queue.pop();

        match result {
            None => {
                backoff.snooze();
                if shutdown.load(Ordering::Relaxed) {
                    return;
                }
                park();
            }
            Some((block_id, sbc)) => {
                queue.insert(block_id, sbc);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;
    use std::io::Seek;
    use std::sync::RwLock;

    #[test]
    pub fn test_add_sequence() {
        let myseq = b"ACTGGGGGGGG".to_vec();

        let test_block_size = 512 * 1024;
        let temp_out: Cursor<Vec<u8>> = Cursor::new(Vec::with_capacity(512 * 1024 * 2));
        // let temp_out = RwLock::new(temp_out);

        let mut sb = SequenceBuffer::default().with_block_size(test_block_size);

        let oq = sb.output_queue();

        let mut locs = Vec::new();

        let loc = sb.add_sequence(&myseq[..]).unwrap();
        locs.extend(loc);

        let mut largeseq = Vec::new();
        while largeseq.len() <= 4 * 1024 * 1024 {
            largeseq.extend_from_slice(&myseq[..]);
        }

        let loc = sb.add_sequence(&largeseq).unwrap();
        locs.extend(loc);

        // println!("Done adding seqs...");

        sb.finalize().expect("Unable to finalize SeqBuffer");

        // println!("Finalized...");

        // This is actually a test, just making sure it does not panic here
        drop(sb);

        assert!(oq.len() == 9);
        let g = oq.pop().unwrap();
        let g = g.1.decompress();
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
        let myseq = b"ACTGGGGGGGG".to_vec();

        let test_block_size = 512 * 1024;

        let mut sb = SequenceBuffer::default().with_block_size(test_block_size);

        sb.add_sequence(&myseq[..]).expect("Error adding sequence");
    }
}
