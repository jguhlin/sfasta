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

use libcompression::*;

pub struct Worker<W: Write + Seek + Send + Seek + 'static>
{
    queue: Option<Arc<ArrayQueue<OutputBlock>>>,
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
        let queue = Arc::new(ArrayQueue::new(self.buffer_size));
        self.queue = Some(Arc::clone(&queue));

        let shutdown = Arc::clone(&self.shutdown_flag);

        let output_buffer = Arc::clone(&self.output_buffer);
        thread::spawn(|| worker(queue, shutdown, output_buffer))
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

    pub fn get_queue(&self) -> Arc<ArrayQueue<OutputBlock>>
    {
        Arc::clone(self.queue.as_ref().unwrap())
    }
}

fn worker<W>(queue: Arc<ArrayQueue<OutputBlock>>, shutdown_flag: Arc<AtomicBool>, output_buffer: Arc<Mutex<W>>)
where
    W: Write + Seek + Send + Sync + Seek,
{
    let backoff = Backoff::new();

    // Hold the mutex for the entire duration
    let output = Arc::clone(&output_buffer);
    let mut output = output.lock().unwrap();

    loop {
        if shutdown_flag.load(Ordering::Relaxed) {
            break;
        }

        if let Some(block) = queue.pop() {
            let location = block.location;
            let data = block.data;

            // Get current location
            let current_location = output.stream_position().unwrap();

            output.write_all(&data).unwrap();
            location.store(current_location, Ordering::Relaxed);
        } else {
            backoff.snooze();
            if backoff.is_completed() {
                thread::sleep(Duration::from_millis(5));
                backoff.reset();
            }
        }
    }
}
