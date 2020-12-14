use std::convert::TryFrom;
use std::io::prelude::*;
use std::io::{BufWriter, SeekFrom};
use std::sync::atomic::Ordering;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::Arc;
use std::thread;
use std::thread::park;

use crossbeam::queue::ArrayQueue;
use crossbeam::utils::Backoff;

use crate::fasta;
use crate::fasta::Fasta;
use crate::io::create_index;
use crate::structs::default_compression_level;
use crate::structs::{
    CompressionType, Entry, EntryCompressedBlock, EntryCompressedHeader, Header, ReadAndSeek,
    WriteAndSeek,
};

/// Converts a FASTA file to an SFASTA file...
/// Internally buffers read and write buffers
// TODO: Split into even smaller subfunctions
// TODO: Pass in number of threads...
pub fn convert_fasta<R, W>(
    in_buf: &'static mut R,
    out_buf: &'static mut W,
    out_idx_buf: &'static mut W,
    citation: Option<String>,
    comment: Option<String>,
    id: Option<String>,
    compression_type: CompressionType,
) where
    R: ReadAndSeek,
    W: WriteAndSeek,
{
    // in_buf will be buffered in the Fasta level
    let mut out_buf = BufWriter::with_capacity(1024 * 1024, out_buf);

    let header = Header {
        citation,
        comment,
        id: id,
        compression_type,
    };

    bincode::serialize_into(&mut out_buf, &header).expect("Unable to write to bincode output");

    let mut ids = Vec::with_capacity(1024 * 1024);
    let mut locations = Vec::with_capacity(1024 * 1024);

    let mut block_ids = Vec::with_capacity(8 * 1024 * 1024);
    let mut block_locations: Vec<u64> = Vec::with_capacity(8 * 1024 * 1024);

    let mut pos = out_buf
        .seek(SeekFrom::Current(0))
        .expect("Unable to work with seek API");

    let fasta = Fasta::from_buffer(in_buf);

    let thread_count;

    if cfg!(test) {
        thread_count = 1;
    } else {
        thread_count = 64;
    }

    let queue_size = 64;

    // multi-threading...
    let shutdown = Arc::new(AtomicBool::new(false));
    let total_entries = Arc::new(AtomicUsize::new(0));
    let compressed_entries = Arc::new(AtomicUsize::new(0));
    let written_entries = Arc::new(AtomicUsize::new(0));

    let queue: Arc<ArrayQueue<fasta::Sequence>> = Arc::new(ArrayQueue::new(queue_size));
    let output_queue: Arc<ArrayQueue<(EntryCompressedHeader, Vec<EntryCompressedBlock>)>> =
        Arc::new(ArrayQueue::new(queue_size));

    let mut worker_handles = Vec::new();

    for _ in 0..thread_count {
        let q = Arc::clone(&queue);
        let oq = Arc::clone(&output_queue);
        let shutdown_copy = Arc::clone(&shutdown);
        let te = Arc::clone(&total_entries);
        let ce = Arc::clone(&compressed_entries);

        let handle = thread::spawn(move || {
            let shutdown = shutdown_copy;
            let backoff = Backoff::new();
            let mut result;
            loop {
                result = q.pop();

                match result {
                    None => {
                        backoff.snooze();
                        if shutdown.load(Ordering::Relaxed)
                            && ce.load(Ordering::Relaxed) == te.load(Ordering::Relaxed)
                        {
                            return;
                        }
                    }
                    Some(x) => {
                        // let x = result.unwrap();
                        let mut entry = generate_sfasta_compressed_entry(
                            x.id.clone(),
                            None,
                            x.seq.to_vec(),
                            CompressionType::LZ4,
                            default_compression_level(CompressionType::LZ4),
                        );

                        while let Err(x) = oq.push(entry) {
                            entry = x;
                            park(); // Queue is full, park the thread...
                        }
                        ce.fetch_add(1, Ordering::SeqCst);
                    }
                }
            }
        });

        worker_handles.push(handle);
    }

    let oq = Arc::clone(&output_queue);
    let shutdown_copy = Arc::clone(&shutdown);
    let q = Arc::clone(&queue);
    let te = Arc::clone(&total_entries);

    // This thread does the writing...
    let output_thread = thread::spawn(move || {
        let shutdown = shutdown_copy;
        let output_queue = oq;
        let backoff = Backoff::new();

        let mut result;
        loop {
            result = output_queue.pop();
            match result {
                None => {
                    // Unpark all other threads..
                    for i in &worker_handles {
                        i.thread().unpark();
                    }
                    backoff.snooze();
                    if (written_entries.load(Ordering::Relaxed) == te.load(Ordering::Relaxed))
                        && shutdown.load(Ordering::Relaxed)
                    {
                        create_index(out_idx_buf, ids, locations, block_ids, block_locations);
                        return;
                    }
                }
                Some(cs) => {
                    ids.push(cs.0.id.clone());

                    locations.push(pos);
                    bincode::serialize_into(&mut out_buf, &cs.0)
                        .expect("Unable to write to bincode output");

                    for block in cs.1 {
                        pos = out_buf
                            .seek(SeekFrom::Current(0))
                            .expect("Unable to work with seek API");
                        block_locations.push(pos);
                        block_ids.push((block.id.clone(), block.block_id));
                        bincode::serialize_into(&mut out_buf, &block)
                            .expect("Unable to write to bincode output");
                    }

                    pos = out_buf
                        .seek(SeekFrom::Current(0))
                        .expect("Unable to work with seek API");
                    written_entries.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
    });

    let backoff = Backoff::new();
    fasta.for_each(|x| {
        // println!("{:#?}", std::str::from_utf8(&x.seq).unwrap());
        let mut item = x;
        while let Err(x) = queue.push(item) {
            item = x;
            backoff.snooze();
        }
        total_entries.fetch_add(1, Ordering::SeqCst);
    });

    while queue.len() > 0 || output_queue.len() > 0 {
        backoff.snooze();
    }

    shutdown.store(true, Ordering::SeqCst);

    output_thread
        .join()
        .expect("Unable to join the output thread back...");
}

// TODO: Remove this code since we have the trait now...
/// Should remove...
fn generate_sfasta_compressed_entry(
    id: String,
    comment: Option<String>,
    seq: Vec<u8>,
    compression_type: CompressionType,
    compression_level: i32,
) -> (EntryCompressedHeader, Vec<EntryCompressedBlock>) {
    let len: u64 =
        u64::try_from(seq.len()).expect("Unlikely as it is, sequence length exceeds u64::MAX...");
    let entry = Entry {
        id,
        seq,
        comment,
        len,
    };

    entry.compress(compression_type, compression_level)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    // TODO: Test compressed...
    #[test]
    pub fn test_convert_fasta() {
        let mut input = Cursor::new(">test\nACTGNANANANANANANANANANATCGGAGACTACGATA\n".to_string().as_bytes().to_vec());
        let mut output: Cursor<Vec<u8>> = Cursor::new(Vec::new());
        let mut idx: Cursor<Vec<u8>> = Cursor::new(Vec::new());

        convert_fasta(&mut input, &mut output, &mut idx, None, None, None, CompressionType::LZ4);
    }
}
