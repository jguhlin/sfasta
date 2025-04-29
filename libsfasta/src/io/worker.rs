//! # I/O Worker
//!
//! This holds the output buffer, must finish and destroy to have
//! access to it again.

use std::{
    io::{Seek, Write},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread,
    thread::JoinHandle,
    time::Duration,
};

use crossbeam::{queue::ArrayQueue, utils::Backoff};
use flume::{Receiver, Sender};

use libcompression::*;

pub struct Worker<W: Write + Seek + Send + Seek + 'static> {
    queue: Option<Arc<Sender<OutputBlock>>>,
    receiver_queue: Option<Receiver<OutputBlock>>,
    shutdown_flag: Arc<AtomicBool>,
    buffer_size: usize,

    // Where to write
    output_buffer: Arc<Mutex<Box<W>>>,
}

impl<W: Write + Seek + Send + Sync + Seek> Worker<W> {
    pub fn into_inner(self) -> Box<W> {
        match Arc::try_unwrap(self.output_buffer) {
            Ok(output_buffer) => output_buffer.into_inner().unwrap(),
            Err(_) => panic!("Could not unwrap output buffer"),
        }
    }

    pub fn new(output_buffer: Arc<Mutex<Box<W>>>) -> Self {
        Self {
            queue: None,
            receiver_queue: None,
            output_buffer,
            buffer_size: 1024,
            shutdown_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn with_buffer_size(mut self, buffer_size: usize) -> Self {
        self.buffer_size = buffer_size;
        self
    }

    pub fn buffer_size(&self) -> usize {
        self.buffer_size
    }

    /// Starts the worker thread, and returns the JoinHandle.
    pub fn start(&mut self) -> JoinHandle<()> {
        let (tx, rx) = flume::bounded(self.buffer_size);
        let queue = Arc::new(tx);
        self.queue = Some(queue);
        self.receiver_queue = Some(rx.clone());

        let shutdown = Arc::clone(&self.shutdown_flag);

        let output_buffer = Arc::clone(&self.output_buffer);
        thread::spawn(|| worker(rx, shutdown, output_buffer))
    }

    /// Manually shutdown the worker
    pub fn shutdown(&mut self) {
        self.shutdown_flag.store(true, Ordering::Relaxed);
    }

    pub fn len(&self) -> usize {
        self.receiver_queue.as_ref().unwrap().len()
    }

    pub fn get_queue(&self) -> Arc<Sender<OutputBlock>> {
        Arc::clone(self.queue.as_ref().unwrap())
    }
}

fn worker<W>(
    queue: Receiver<OutputBlock>,
    shutdown_flag: Arc<AtomicBool>,
    output_buffer: Arc<Mutex<W>>,
) where
    W: Write + Seek + Send + Sync + Seek,
{
    let bincode_config = bincode::config::standard().with_fixed_int_encoding();

    // Hold the mutex for the entire duration
    let output = Arc::clone(&output_buffer);
    let mut output = output.lock().unwrap();

    let mut output_locs: Vec<Arc<AtomicU64>> = Vec::new();
    let mut output_buffer = Vec::new();

    loop {
        let iter = queue.drain();
        iter.for_each(|block| {
            output_locs.push(block.location);
            output_buffer.push(block.data);
        });

        if !output_buffer.is_empty() {
            // Get current location
            output_buffer.iter().enumerate().for_each(|(i, x)| {
                let loc = &output_locs[i];
                loc.store(output.stream_position().unwrap(), Ordering::Relaxed);
                bincode::encode_into_std_write(
                    &x,
                    &mut *output,
                    bincode_config,
                )
                .unwrap();
            });

            output_buffer.clear();
            output_locs.clear();
        }

        if shutdown_flag.load(Ordering::Relaxed) {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_worker() {
        let output_buffer =
            Arc::new(Mutex::new(Box::new(Cursor::new(Vec::new()))));
        let mut worker = Worker::new(output_buffer);
        let mut worker = worker.with_buffer_size(8192);
        assert!(worker.buffer_size() == 8192);

        let handle = worker.start();

        let queue = worker.get_queue();
        let location = Arc::new(AtomicU64::new(0));
        queue
            .send(OutputBlock {
                data: vec![1, 2, 3],
                location: Arc::clone(&location),
            })
            .unwrap();

        worker.shutdown();
        handle.join().unwrap();

        println!("Location: {}", location.load(Ordering::Relaxed));

        let output_buffer = worker.into_inner();
        let output_buffer = output_buffer.into_inner();
        assert_eq!(output_buffer, vec![3, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3]);
    }
}
