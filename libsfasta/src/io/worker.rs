//! # I/O Worker
//! 
//! This holds the output buffer, must finish and destroy to have access to it again.

use std::io::{Write, Seek};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam::queue::ArrayQueue;
use crossbeam::utils::Backoff;

pub struct OutputBlock {
    pub data: Vec<u8>,
    pub location: Arc<AtomicU64>,
}

pub struct Worker<W: Write + Seek> {
    queue: Option<Arc<ArrayQueue<OutputBlock>>>,
    buffer_size: usize,

    // Where to write
    output_buffer: W
}

impl<W: Write + Seek> Worker<W> {
    pub fn into_inner(self) -> W {
        self.output_buffer
    }

    pub fn new(output_buffer: W) -> Self {
        Self {
            queue: None,
            output_buffer,
            buffer_size: 1024,
        }
    }

    pub fn with_buffer_size(mut self, buffer_size: usize) -> Self {
        self.buffer_size = buffer_size;
        self
    }

    pub fn start(&mut self) {
        let queue = Arc::new(ArrayQueue::new(self.buffer_size));
        self.queue = Some(queue.clone());

        
    }

    
}




#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_worker() {
        let mut worker = Worker::new(Cursor::new(Vec::new()));
        worker.output_buffer.write_all(b"Hello World!").unwrap();
        let output_buffer = worker.into_inner();
        assert_eq!(output_buffer.into_inner(), b"Hello World!");
    }

    // Test can use BufWriter
    #[test]
    fn test_worker_bufwriter() {
        let output_buffer = std::io::BufWriter::new(Cursor::new(Vec::new()));
        let mut worker = Worker::new(output_buffer);
        worker.output_buffer.write_all(b"Hello World!").unwrap();
        let output_buffer = worker.into_inner().into_inner().unwrap().into_inner();
        assert_eq!(output_buffer, b"Hello World!");
    }

    // Test uses u8
    #[test]
    fn test_worker_u8() {
        let mut worker = Worker::new(Cursor::new(Vec::new()));
        worker.output_buffer.write_all(&[0, 1, 2, 3, 4, 5]).unwrap();
        let output_buffer = worker.into_inner();
        assert_eq!(output_buffer.into_inner(), &[0, 1, 2, 3, 4, 5]);
    }
}

