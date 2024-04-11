//! # I/O Worker
//!
//! This holds the output buffer, must finish and destroy to have access to it again.

use std::{
    io::{Seek, Write},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Condvar, Mutex,
    },
    thread,
    thread::JoinHandle,
    time::Duration,
};

use crossbeam::{queue::ArrayQueue, utils::Backoff};
use flume::{Sender, Receiver};

use libcompression::*;

pub struct Worker<W: Write + Seek + Send + Seek + 'static>
{
    queue: Option<Arc<Sender<OutputBlock>>>,
    shutdown_flag: Arc<AtomicBool>,
    buffer_size: usize,

    // Where to write
    output_buffer: Arc<Mutex<Box<W>>>,
}

impl<W: Write + Seek + Send + Sync + Seek> Worker<W>
{
    pub fn into_inner(self) -> Box<W>
    {
        match Arc::try_unwrap(self.output_buffer) {
            Ok(output_buffer) => output_buffer.into_inner().unwrap(),
            Err(_) => panic!("Could not unwrap output buffer"),
        }
    }

    pub fn new(output_buffer: Arc<Mutex<Box<W>>>) -> Self
    {
        Self {
            queue: None,
            output_buffer,
            buffer_size: 1024,
            shutdown_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn with_buffer_size(mut self, buffer_size: usize) -> Self
    {
        self.buffer_size = buffer_size;
        self
    }

    /// Starts the worker thread, and returns the JoinHandle.
    pub fn start(&mut self) -> JoinHandle<()>
    {
        let (tx, rx) = flume::bounded(self.buffer_size);
        let queue = Arc::new(tx);
        self.queue = Some(queue);
        // let queue = Arc::new(ArrayQueue::new(self.buffer_size));
        // self.queue = Some(Arc::clone(&queue));

        let shutdown = Arc::clone(&self.shutdown_flag);

        let output_buffer = Arc::clone(&self.output_buffer);
        thread::spawn(|| worker(rx, shutdown, output_buffer))
    }

    /// Manually shutdown the worker
    pub fn shutdown(&mut self)
    {
        self.shutdown_flag.store(true, Ordering::Relaxed);
    }

    pub fn len(&self) -> usize
    {
        self.queue.as_ref().unwrap().len()
    }

    pub fn get_queue(&self) -> Arc<Sender<OutputBlock>>
    {
        Arc::clone(self.queue.as_ref().unwrap())
    }
}

fn worker<W>(queue: Receiver<OutputBlock>, shutdown_flag: Arc<AtomicBool>, output_buffer: Arc<Mutex<W>>)
where
    W: Write + Seek + Send + Sync + Seek,
{
    let backoff = Backoff::new();
    let bincode_config = bincode::config::standard().with_fixed_int_encoding();

    // Hold the mutex for the entire duration
    let output = Arc::clone(&output_buffer);
    let mut output = output.lock().unwrap();

    let mut output_lens = Vec::new();
    let mut output_locs: Vec<Arc<AtomicU64>> = Vec::new();
    let mut output_buffer = Vec::new();

    loop {

        if !output_buffer.is_empty() {
            // Get current location
            let current_location = output.stream_position().unwrap();
            output_lens.iter_mut().for_each(|x| *x += current_location);
            bincode::encode_into_std_write(&output_buffer, &mut *output, bincode_config).unwrap();
            output_lens.iter().zip(output_locs.iter()).for_each(|(len, loc)| {
                loc.store(*len, Ordering::Relaxed);
            });

            output_buffer.clear();
            output_lens.clear();
            output_locs.clear();
        }

        if shutdown_flag.load(Ordering::Relaxed) {
            break;
        }

        let iter = queue.drain();
        iter.for_each(|block| {
            output_locs.push(block.location);
            output_lens.push(block.data.len() as u64);
            output_buffer.push(block.data);
        });
    }
}
