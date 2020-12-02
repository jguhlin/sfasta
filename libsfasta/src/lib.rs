extern crate bincode;
extern crate bumpalo;
extern crate crossbeam;
extern crate flate2;
extern crate lz4;
extern crate serde;
extern crate serde_bytes;
extern crate rand;
extern crate snap;
extern crate thincollections;
extern crate zstd;

use hashbrown::HashMap;
use lz4::{Decoder, EncoderBuilder};
use std::convert::From;
use std::convert::TryFrom;
use std::fs::{metadata, File};
use std::io::prelude::*;
use std::io::{BufReader, BufWriter, Read, SeekFrom};
use std::path::Path;
use std::time::Instant;

mod fasta;
mod io;
mod utils;

use crate::utils::generic_open_file;

use rand::prelude::*;
use rand_chacha::ChaCha20Rng;


use serde::{Deserialize, Serialize};

use std::sync::{Arc, RwLock};

// SuperTrait
pub trait ReadAndSeek: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadAndSeek for T {}

// TODO: Spin this out as a separate library...
// TODO: Set a const for BufReader buffer size
//       Make it a global const, but also maybe make it configurable?
//       Reason being that network FS will benefit from larger buffers
// TODO: Also make BufWriter bufsize global, but ok to leave larger.
#[derive(PartialEq, Serialize, Deserialize, Debug, Clone)]
pub enum CompressionType {
    ZSTD,   // 19 should be default compression ratio
    LZ4,    // 9 should be default compression ratio
    SNAPPY, // Not yet implemented -- IMPLEMENT
    GZIP,   // Please don't use this -- IMPLEMENT
    NAF,    // Not yet supported -- IMPLEMENT
    NONE,   // No Compression -- IMPLEMENT
}

const fn default_compression_level(ct: CompressionType) -> i32 {
    match ct {
        CompressionType::ZSTD => 19,
        CompressionType::LZ4 => 9,
        _ => 3,
    }
}

#[derive(PartialEq)]
pub enum SeqMode {
    Linear,
    Random,
}

#[derive(PartialEq, Serialize, Deserialize, Debug, Clone)]
pub struct Header {
    pub id: Option<String>,
    pub comment: Option<String>,
    pub citation: Option<String>,
    pub compression_type: CompressionType,
}

/// Represents an entry from an SFASTA file (extension of .sfasta)
#[derive(PartialEq, Serialize, Deserialize, Debug)]
pub struct Entry {
    pub id: String,
    #[serde(with = "serde_bytes")]
    pub seq: Vec<u8>,
    pub comment: Option<String>,
    pub len: u64,
}

const BLOCK_SIZE: usize = 8 * 1024 * 1024;

impl Entry {
    pub fn compress(
        self,
        compression_type: CompressionType,
        compression_level: i32,
    ) -> (EntryCompressedHeader, Vec<EntryCompressedBlock>) {
        let Entry {
            id,
            seq,
            comment,
            len,
        } = self;

        let mut block_count = 0;
        let mut compressed_blocks: Vec<EntryCompressedBlock> = Vec::new();

        let blocks = seq.chunks(BLOCK_SIZE);

        match compression_type {
            CompressionType::ZSTD => {
                let mut compressor = zstd::block::Compressor::new();

                for block in blocks {
                    let compressed = compressor
                        .compress(&block, compression_level)
                        .expect("Unable to write to compression buffer");
                    compressed_blocks.push(EntryCompressedBlock {
                        id: id.clone(),
                        block_id: block_count,
                        compressed_seq: compressed,
                    });

                    block_count = match block_count.checked_add(1) {
                        Some(x) => x,
                        None => panic!("That's too many blocks...."),
                    };
                }
            }
            CompressionType::LZ4 => {
                for block in blocks {
                    let mut output_buffer = Vec::new();
                    let mut encoder = EncoderBuilder::new()
                        .level(9)
                        .build(output_buffer)
                        .expect("Unable to create LZ4 encoder");

                    encoder
                        .write(block)
                        .expect("Unable to write to compression buffer");
                    let output_buffer = encoder.finish().0;

                    compressed_blocks.push(EntryCompressedBlock {
                        id: id.clone(),
                        block_id: block_count,
                        compressed_seq: output_buffer,
                    });

                    block_count = match block_count.checked_add(1) {
                        Some(x) => x,
                        None => panic!("That's too many blocks...."),
                    };
                }
            }
            _ => panic!("Unsupported compression"),
        };

        (
            EntryCompressedHeader {
                id,
                compression_type,
                block_count,
                comment,
                len,
            },
            compressed_blocks,
        )
    }
}

/// SFASTA files stored on disk are bincoded, with the sequence being
/// stored in separate entries following the EntryCompressedHeader.
/// This allows faster searching of SFASTA files without spending
/// CPU cycles on decompression prematurely.
#[derive(PartialEq, Serialize, Deserialize, Clone)]
pub struct EntryCompressedHeader {
    pub id: String,
    pub compression_type: CompressionType,
    pub block_count: u64,
    pub comment: Option<String>,
    pub len: u64,
}

#[derive(PartialEq, Serialize, Deserialize, Clone)]
pub struct EntryCompressedBlock {
    pub id: String,
    pub block_id: u64,
    #[serde(with = "serde_bytes")]
    pub compressed_seq: Vec<u8>,
}

/// Converts an SFASTA::Entry into io::Sequence for further processing
/// Really an identity function...
impl From<Entry> for io::Sequence {
    fn from(item: Entry) -> Self {
        let len = item.seq.len();
        io::Sequence {
            id: item.id,
            seq: item.seq,
            location: 0,
            end: len,
        }
    }
}

/// Iterator to return sfasta::EntryCompressed
pub struct CompressedSequences {
    pub header: Header,
    reader: Box<dyn ReadAndSeek + Send>,
    pub idx: Option<(Vec<String>, Vec<u64>, Vec<(String, u64)>, Vec<u64>)>,
    access: SeqMode,
    random_list: Option<Vec<u64>>,
    filesize: u64,
}

/// Iterator to return io::Sequences
pub struct Sequences {
    pub header: Header,
    reader: Box<dyn ReadAndSeek + Send>,
    pub idx: Option<(Vec<String>, Vec<u64>, Vec<(String, u64)>, Vec<u64>)>,
    access: SeqMode,
    random_list: Option<Vec<u64>>,
    filesize: u64,
}

impl Sequences {
    /// Given a filename, returns a Sequences variable.
    /// Can be used as an iterator.
    pub fn new(filename: &str) -> Sequences {
        let filesize = metadata(&filename).expect("Unable to open file").len();
        let (mut reader, idx) = open_file(filename);
        let header: Header = match bincode::deserialize_from(&mut reader) {
            Ok(x) => x,
            Err(_) => panic!("Header missing or malformed in SFASTA file"),
        };

        Sequences {
            header,
            reader,
            idx,
            access: SeqMode::Linear,
            random_list: None,
            filesize,
        }
    }

    pub fn set_mode(&mut self, access: SeqMode) {
        self.access = access;

        if self.access == SeqMode::Random {
            assert!(self.idx.is_some());
            let idx = self.idx.as_ref().unwrap();
            let mut locs: Vec<u64> = idx.1.clone();
            let mut rng = ChaCha20Rng::seed_from_u64(42);
            locs.shuffle(&mut rng);
            self.random_list = Some(locs);
        } else {
            self.random_list = None;
        }
    }

    pub fn get(&mut self, id: &str) -> Option<io::Sequence> {
        let pos = match self.idx.as_ref().unwrap().0.binary_search(&id.to_string()) {
            Ok(x) => x,
            Err(_) => return None,
        };

        let file_pos = self.idx.as_ref().unwrap().1[pos].clone();
        self.get_at(file_pos)
    }

    pub fn get_header_at(&mut self, file_pos: u64) -> Result<EntryCompressedHeader, &'static str> {
        self.reader
            .seek(SeekFrom::Start(file_pos))
            .expect("Unable to work with seek API");

        let ec: EntryCompressedHeader = match bincode::deserialize_from(&mut self.reader) {
            Ok(x) => x,
            Err(_) => return Err("Unable to get header via get_header_at"),
        };

        Ok(ec)
    }

    pub fn get_seq_slice(
        &mut self,
        id: String,
        start: u64,
        end: u64,
    ) -> Result<Vec<u8>, &'static str> {
        let start_block = start as f64 / BLOCK_SIZE as f64;
        let start_block = start_block.floor() as usize;

        let end_block = end as f64 / BLOCK_SIZE as f64;
        let end_block = end_block.ceil() as usize;

        let mut sequence: Vec<u8> = Vec::with_capacity((end_block - start_block) * BLOCK_SIZE);

        match self.header.compression_type {
            CompressionType::ZSTD => {
                let mut decompressor = zstd::block::Decompressor::new();

                for i in start_block..end_block {
                    let block_loc = match self
                        .idx
                        .as_ref()
                        .unwrap()
                        .2
                        .binary_search(&(id.clone(), i as u64))
                    {
                        Ok(x) => self.idx.as_ref().unwrap().3[x] as u64,
                        Err(_) => return Err("Unable to find block"),
                    };

                    self.reader
                        .seek(SeekFrom::Start(block_loc))
                        .expect("Unable to work with seek API");

                    let ec: EntryCompressedBlock = match bincode::deserialize_from(&mut self.reader)
                    {
                        Ok(x) => x,
                        Err(_) => return Err("Unable to get block in get_seq_slice"),
                    };

                    let seq = match decompressor.decompress(&ec.compressed_seq, BLOCK_SIZE) {
                        Ok(x) => x,
                        Err(_) => panic!("Unable to decompress"),
                    };

                    sequence.extend_from_slice(&seq);
                }
            }
            CompressionType::LZ4 => {
                // let mut decompression_buffer = Vec::new();

                for i in start_block..end_block {
                    let block_loc = match self
                        .idx
                        .as_ref()
                        .unwrap()
                        .2
                        .binary_search(&(id.clone(), i as u64))
                    {
                        Ok(x) => self.idx.as_ref().unwrap().3[x] as u64,
                        Err(_) => return Err("Unable to find block"),
                    };

                    self.reader
                        .seek(SeekFrom::Start(block_loc))
                        .expect("Unable to work with seek API");

                    let ec: EntryCompressedBlock = match bincode::deserialize_from(&mut self.reader)
                    {
                        Ok(x) => x,
                        Err(_) => return Err("Unable to get block in get_seq_slice"),
                    };

                    let mut decompressor =
                        Decoder::new(&ec.compressed_seq[..]).expect("Unable to create LZ4 decoder");
                    let mut output = Vec::new();
                    decompressor.read_to_end(&mut output);
                    sequence.extend_from_slice(&output[..]);
                    decompressor.finish();
                }
            }
            _ => panic!("Unsupported compression type"),
        }

        let start = start as usize - (BLOCK_SIZE as usize * start_block);
        let end = end as usize - (BLOCK_SIZE as usize * start_block);

        return Ok(sequence[start..end].to_vec());
    }

    // Decompresses the entire sequence...
    pub fn get_at(&mut self, file_pos: u64) -> Option<io::Sequence> {
        self.reader
            .seek(SeekFrom::Start(file_pos))
            .expect("Unable to work with seek API");

        let ec: EntryCompressedHeader = match bincode::deserialize_from(&mut self.reader) {
            Ok(x) => x,
            Err(_) => return None, // panic!("Error at SFASTA::Sequences::next: {}", y)
        };

        let mut sequence = Vec::new();

        match self.header.compression_type {
            CompressionType::ZSTD => {
                let mut decompressor = zstd::block::Decompressor::new();

                for _i in 0..ec.block_count as usize {
                    let entry: EntryCompressedBlock =
                        match bincode::deserialize_from(&mut self.reader) {
                            Ok(x) => x,
                            Err(_) => panic!("Error decoding compressed block"),
                        };

                    let seq = match decompressor.decompress(&entry.compressed_seq, BLOCK_SIZE) {
                        Ok(x) => x,
                        Err(_) => panic!("Unable to decompress"),
                    };

                    sequence.extend_from_slice(&seq);
                }
            }
            CompressionType::LZ4 => {
                for _i in 0..ec.block_count as usize {
                    let entry: EntryCompressedBlock =
                        match bincode::deserialize_from(&mut self.reader) {
                            Ok(x) => x,
                            Err(_) => panic!("Error decoding compressed block"),
                        };

                    let mut decompressor = Decoder::new(&entry.compressed_seq[..])
                        .expect("Unable to create LZ4 decoder");
                    let mut output = Vec::new();
                    decompressor.read_to_end(&mut output);
                    sequence.extend_from_slice(&output[..]);
                    decompressor.finish();
                }
            }
            _ => panic!("Unsupported compression type"),
        };

        sequence.make_ascii_uppercase();

        Some(io::Sequence {
            seq: sequence,
            end: ec.len as usize,
            id: ec.id,
            location: 0,
        })
    }

    // Convert to iterator that only returns EntryCompressed...
    pub fn into_compressed_sequences(self) -> CompressedSequences {
        let Sequences {
            header,
            reader,
            idx,
            access,
            random_list,
            filesize,
        } = self;

        CompressedSequences {
            header,
            reader,
            idx,
            access,
            random_list,
            filesize,
        }
    }
}

// TODO: Create the option to pass back lowercase stuff too..
// Maybe for repeat masking and such? Right now it's all uppercase.
impl Iterator for Sequences {
    type Item = io::Sequence;

    /// Get the next SFASTA entry as io::Sequence type
    fn next(&mut self) -> Option<io::Sequence> {
        if self.access == SeqMode::Random {
            assert!(self.random_list.is_some());
            let rl = self.random_list.as_mut().unwrap();
            let next_loc = match rl.pop() {
                Some(x) => x,
                None => return None,
            };

            self.reader
                .seek(SeekFrom::Start(next_loc))
                .expect("Unable to work with seek API");
        } else {
            let pos = self
                .reader
                .seek(SeekFrom::Current(0))
                .expect("Unable to work with seek API");

            if pos == self.filesize {
                return None;
            }
        }

        let ec: EntryCompressedHeader = match bincode::deserialize_from(&mut self.reader) {
            Ok(x) => x,
            Err(y) => panic!("Error at SFASTA::Sequences::next: {:#?}", y),
        };

        let mut sequence = Vec::new();

        for _i in 0..ec.block_count as usize {
            let entry: EntryCompressedBlock = match bincode::deserialize_from(&mut self.reader) {
                Ok(x) => x,
                Err(_) => panic!("Error decoding compressed block"),
            };

            match self.header.compression_type {
                CompressionType::ZSTD => {
                    let mut decompressor = zstd::block::Decompressor::new();
                    let seq = match decompressor.decompress(&entry.compressed_seq, BLOCK_SIZE) {
                        Ok(x) => x,
                        Err(_) => panic!("Unable to decompress"),
                    };

                    sequence.extend_from_slice(&seq);
                }

                CompressionType::LZ4 => {
                    let mut decompressor = Decoder::new(&entry.compressed_seq[..])
                        .expect("Unable to create LZ4 decoder");
                    let mut output = Vec::new();
                    decompressor.read_to_end(&mut output);
                    sequence.extend_from_slice(&output[..]);
                    &decompressor.finish();
                }
                _ => panic!("Unsupported compression type"),
            };
        }

        sequence.make_ascii_uppercase();

        Some(io::Sequence {
            seq: sequence,
            end: ec.len as usize,
            id: ec.id,
            location: 0,
        })
    }
}

impl Iterator for CompressedSequences {
    type Item = (EntryCompressedHeader, Vec<EntryCompressedBlock>);

    /// Get the next SFASTA entry as an EntryCompressed struct
    fn next(&mut self) -> Option<(EntryCompressedHeader, Vec<EntryCompressedBlock>)> {
        if self.access == SeqMode::Random {
            assert!(self.random_list.is_some());
            let rl = self.random_list.as_mut().unwrap();
            let next_loc = rl.pop();
            next_loc?;
            self.reader
                .seek(SeekFrom::Start(next_loc.unwrap()))
                .expect("Unable to work with seek API");
        } else {
            let pos = self
                .reader
                .seek(SeekFrom::Current(0))
                .expect("Unable to work with seek API");

            if pos == self.filesize {
                return None;
            }
        }

        let ec: EntryCompressedHeader = match bincode::deserialize_from(&mut self.reader) {
            Ok(x) => x,
            Err(_) => return None, // panic!("Error at SFASTA::Sequences::next: {}", y)
        };

        let mut blocks = Vec::with_capacity(ec.block_count as usize);
        for _i in 0..ec.block_count as usize {
            let x: EntryCompressedBlock = bincode::deserialize_from(&mut self.reader).unwrap();
            blocks.push(x);
        }

        // Have to convert from EntryCompressed to Entry, this handles that middle
        // conversion.
        Some((ec, blocks))
    }
}

// sfasta is:
// bincode encoded
//   fasta ID
//   zstd compressed sequence

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

/// Opens an SFASTA file and an index and returns a Box<dyn Read>,
/// HashMap<String, usize> type
pub fn open_file(
    filename: &str,
) -> (
    Box<dyn ReadAndSeek + Send>,
    Option<(Vec<String>, Vec<u64>, Vec<(String, u64)>, Vec<u64>)>,
) {
    let filename = check_extension(filename);

    let file = match File::open(Path::new(&filename)) {
        Err(_) => panic!("Couldn't open {}", filename),
        Ok(file) => file,
    };

    let reader = BufReader::with_capacity(512 * 1024, file);

    (Box::new(reader), load_index(&filename))
}

use crossbeam::queue::ArrayQueue;
use crossbeam::utils::Backoff;
use std::sync::atomic::Ordering;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::thread;
use std::thread::park;

/// Converts a FASTA file to an SFASTA file...
pub fn convert_fasta_file(filename: &str, output: &str)
// TODO: Make multithreaded for very large datasets (>= 1Gbp or 5Gbp or something)
// TODO: Add progress bar option ... or not..
//
// Convert file to bincode/zstd for faster processing
// Stores accession/taxon information inside the Sequence struct
{
    let output_filename = check_extension(output);

    let out_file = File::create(output_filename.clone()).expect("Unable to write to file");
    let mut out_fh = BufWriter::with_capacity(1024 * 1024, out_file);

    let (filesize, _, _) = generic_open_file(filename);

    let header = Header {
        citation: None,
        comment: None,
        id: Some(filename.to_string()),
        compression_type: CompressionType::LZ4,
    };

    bincode::serialize_into(&mut out_fh, &header).expect("Unable to write to bincode output");

    let starting_size = std::cmp::max((filesize / 500) as usize, 1024);

    let mut ids = Vec::with_capacity(starting_size);
    let mut locations = Vec::with_capacity(starting_size);

    let mut block_ids = Vec::with_capacity(starting_size * 1024);
    let mut block_locations: Vec<u64> = Vec::with_capacity(starting_size * 1024);

    let mut pos = out_fh
        .seek(SeekFrom::Current(0))
        .expect("Unable to work with seek API");

    let fasta = fasta::Fasta::new(filename);

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
                        drop(out_fh);
                        create_index(&output_filename, ids, locations, block_ids, block_locations);
                        return;
                    }
                }
                Some(cs) => {
                    ids.push(cs.0.id.clone());

                    locations.push(pos);
                    bincode::serialize_into(&mut out_fh, &cs.0)
                        .expect("Unable to write to bincode output");

                    for block in cs.1 {
                        pos = out_fh
                            .seek(SeekFrom::Current(0))
                            .expect("Unable to work with seek API");
                        block_locations.push(pos);
                        block_ids.push((block.id.clone(), block.block_id));
                        bincode::serialize_into(&mut out_fh, &block)
                            .expect("Unable to write to bincode output");
                    }

                    pos = out_fh
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

/// Get all IDs from an SFASTA file
/// Really a debugging function...
pub fn get_headers_from_sfasta(filename: String) -> Vec<String> {
    let file = match File::open(&filename) {
        Err(why) => panic!("Couldn't open {}: {}", filename, why.to_string()),
        Ok(file) => file,
    };

    let mut reader = BufReader::with_capacity(512 * 1024, file);

    let header: Header = match bincode::deserialize_from(&mut reader) {
        Ok(x) => x,
        Err(_) => panic!("Header missing or malformed in SFASTA file"),
    };

    let mut ids: Vec<String> = Vec::with_capacity(2048);

    while let Ok(entry) = bincode::deserialize_from::<_, EntryCompressedHeader>(&mut reader) {
        ids.push(entry.id);
        for _i in 0..entry.block_count as usize {
            let _x: EntryCompressedBlock = bincode::deserialize_from(&mut reader).unwrap();
        }
    }

    ids
}

/// Get all IDs from an SFASTA file
/// Really a debugging function...
pub fn test_sfasta(filename: String) {
    let file = match File::open(&filename) {
        Err(why) => panic!("Couldn't open {}: {}", filename, why.to_string()),
        Ok(file) => file,
    };

    let filesize = metadata(&filename).expect("Unable to open file").len();

    let mut reader = BufReader::with_capacity(512 * 1024, file);

    let mut seqnum: usize = 0;

    let header: Header = match bincode::deserialize_from(&mut reader) {
        Ok(x) => x,
        Err(_) => panic!("Header missing or malformed in SFASTA file"),
    };

    let mut pos = 0;

    loop {
        pos = reader
            .seek(SeekFrom::Current(0))
            .expect("Unable to work with seek API");

        if pos == filesize {
            break;
        }

        seqnum += 1;
        let entry: EntryCompressedHeader = match bincode::deserialize_from(&mut reader) {
            Ok(x) => x,
            Err(x) => panic!("Found error: {}", x),
        };

        for _i in 0..entry.block_count as usize {
            let _x: EntryCompressedBlock = bincode::deserialize_from(&mut reader).unwrap();
        }
    }
}

/// Checks that the file extension ends in .sfasta or adds it if necessary
fn check_extension(filename: &str) -> String {
    if !filename.ends_with(".sfasta") {
        format!("{}.sfasta", filename)
    } else {
        filename.to_string()
    }
}

/// Indexes an SFASTA file
pub fn index(filename: &str) -> String {
    // TODO: Run a sanity check on the file first... Make sure it's valid
    // sfasta

    let filesize = metadata(&filename).expect("Unable to open file").len();
    let starting_size = std::cmp::max((filesize / 500) as usize, 1024);

    //    let mut idx: HashMap<String, u64, RandomXxHashBuilder64> =
    // Default::default();    idx.reserve(starting_size);

    let mut ids = Vec::with_capacity(starting_size);
    let mut locations = Vec::with_capacity(starting_size);

    let mut block_ids = Vec::with_capacity(starting_size);
    let mut block_locations = Vec::with_capacity(starting_size);

    let fh = match File::open(&filename) {
        Err(why) => panic!("Couldn't open {}: {}", filename, why.to_string()),
        Ok(file) => file,
    };

    let mut fh = BufReader::with_capacity(512 * 1024, fh);
    let mut pos = fh
        .seek(SeekFrom::Current(0))
        .expect("Unable to work with seek API");
    let mut i = 0;
    let mut now = Instant::now();
    //    let mut bump = Bump::new();
    //    let mut maxalloc: usize = 0;

    let header: Header = match bincode::deserialize_from(&mut fh) {
        Ok(x) => x,
        Err(_) => panic!("Header missing or malformed in SFASTA file"),
    };

    while let Ok(entry) = bincode::deserialize_from::<_, EntryCompressedHeader>(&mut fh) {
        i += 1;
        if i % 100_000 == 0 {
            println!("100k at {} ms.", now.elapsed().as_millis()); //Maxalloc {} bytes", now.elapsed().as_secs(), maxalloc);
            println!(
                "{}/{} {}",
                pos,
                filesize,
                (pos as f32 / filesize as f32) as f32
            );
            now = Instant::now();
        }

        ids.push(entry.id.clone());
        locations.push(pos);

        pos = fh
            .seek(SeekFrom::Current(0))
            .expect("Unable to work with seek API");

        for i in 0..entry.block_count as usize {
            pos = fh
                .seek(SeekFrom::Current(0))
                .expect("Unable to work with seek API");

            let entry: EntryCompressedBlock = match bincode::deserialize_from(&mut fh) {
                Ok(x) => x,
                Err(_) => panic!("Error decoding compressed block"),
            };

            block_ids.push((entry.id.clone(), i as u64));
            block_locations.push(pos as u64);
        }

        pos = fh
            .seek(SeekFrom::Current(0))
            .expect("Unable to work with seek API");
    }

    create_index(filename, ids, locations, block_ids, block_locations)
}

fn get_index_filename(filename: &str) -> String {
    let filenamepath = Path::new(&filename);
    let filename = Path::new(filenamepath.file_name().unwrap())
        .file_stem()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned()
        + ".sfai";

    let mut path = filenamepath.parent().unwrap().to_str().unwrap().to_owned();
    if !path.is_empty() {
        path += "/";
    }

    path + &filename
}

pub fn clear_idxcache() {
 //   IDXCACHE.get_or_init(|| Arc::new(RwLock::new(HashMap::new())));

//    let mut idxcache = IDXCACHE.get().unwrap().write().unwrap();
//    *idxcache = HashMap::new();
}

pub fn load_index(filename: &str) -> Option<(Vec<String>, Vec<u64>, Vec<(String, u64)>, Vec<u64>)> {
//    IDXCACHE.get_or_init(|| Arc::new(RwLock::new(HashMap::new())));

    let idx_filename = get_index_filename(filename);

/*    if IDXCACHE
        .get()
        .unwrap()
        .read()
        .unwrap()
        .contains_key(&idx_filename)
    {
        return Some(
            IDXCACHE
                .get()
                .unwrap()
                .read()
                .unwrap()
                .get(&idx_filename)
                .unwrap()
                .clone(),
        );
    }*/

    if !Path::new(&idx_filename).exists() {
        println!("IdxFile does not exist! {} {}", filename, idx_filename);
        return None;
    }

    let (_, _, mut idxfh) = generic_open_file(&idx_filename);
    // let idx: HashMap<String, u64>;
    // let idx: HashMap<String, u64>;
    // idx = bincode::deserialize_from(&mut idxfh).expect("Unable to open Index
    // file");
    let _length: u64 =
        bincode::deserialize_from(&mut idxfh).expect("Unable to read length of index");
    let keys: Vec<String> = bincode::deserialize_from(&mut idxfh).expect("Unable to read idx keys");
    let vals: Vec<u64> = bincode::deserialize_from(&mut idxfh).expect("Unable to read idx values");

    let block_keys: Vec<(String, u64)> =
        bincode::deserialize_from(&mut idxfh).expect("Unable to read idx keys");
    let block_vals: Vec<u64> =
        bincode::deserialize_from(&mut idxfh).expect("Unable to read idx values");

/*    let mut idxcache = IDXCACHE.get().unwrap().write().unwrap();
    idxcache.insert(
        idx_filename.clone(),
        (
            keys.clone(),
            vals.clone(),
            block_keys.clone(),
            block_vals.clone(),
        ),
    ); */

    Some((keys, vals, block_keys, block_vals))
}

pub fn create_index(
    filename: &str,
    ids: Vec<String>,
    locations: Vec<u64>,
    block_ids: Vec<(String, u64)>,
    block_locations: Vec<u64>,
) -> String {
    let idx: HashMap<String, u64> = ids.into_iter().zip(locations).collect();

    let mut sorted: Vec<_> = idx.into_iter().collect();
    sorted.sort_by(|x, y| x.0.cmp(&y.0));
    let keys: Vec<_> = sorted.iter().map(|x| x.0.clone()).collect();
    let vals: Vec<_> = sorted.iter().map(|x| x.1.clone()).collect();

    let output_filename = get_index_filename(filename);

    let out_file = snap::write::FrameEncoder::new(
        File::create(output_filename.clone()).expect("Unable to write to file"),
    );

    let mut out_fh = BufWriter::with_capacity(1024 * 1024, out_file);
    // bincode::serialize_into(&mut out_fh, &idx).expect("Unable to write index");

    bincode::serialize_into(&mut out_fh, &(keys.len() as u64)).expect("Unable to write index");
    bincode::serialize_into(&mut out_fh, &keys).expect("Unable to write index");
    bincode::serialize_into(&mut out_fh, &vals).expect("Unable to write index");

    let idx: HashMap<(String, u64), u64> = block_ids.into_iter().zip(block_locations).collect();

    let mut sorted: Vec<_> = idx.into_iter().collect();
    sorted.sort_by(|x, y| x.0.cmp(&y.0));
    let keys: Vec<_> = sorted.iter().map(|x| x.0.clone()).collect();
    let vals: Vec<_> = sorted.iter().map(|x| x.1.clone()).collect();
    bincode::serialize_into(&mut out_fh, &keys).expect("Unable to write index");
    bincode::serialize_into(&mut out_fh, &vals).expect("Unable to write index");

    output_filename
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::SeekFrom;

    #[test]
    pub fn test_entry() {
        let seq = b"ACTGGACTACAGTTCAGGACATCACTTTCACTACTAGTGAGATTGACCACTA".to_vec();
        let oseq = seq.clone();
        let e = Entry {
            id: "Test".to_string(),
            len: seq.len() as u64,
            seq: seq,
            comment: None,
        };

        let ec = e.compress(CompressionType::LZ4, 9);
        //        let e = ec.decompress();

        //        assert!(e.seq == oseq);
    }

    #[test]
    pub fn convert_fasta_to_sfasta_and_index() {
        let input_filename = "test_data/test_multiple.fna";
        let output_filename = "test_data/test_sfasta_convert_and_index.sfasta";

        clear_idxcache();
        convert_fasta_file(input_filename, output_filename);

        let idx_filename = index(output_filename);
        assert!(idx_filename == "test_data/test_sfasta_convert_and_index.sfai");

        load_index(output_filename);
    }

    #[test]
    pub fn test_sfasta_fn() {
        let input_filename = "test_data/test_large_multiple.fna";
        let output_filename = "test_data/test_sfasta_fn.sfasta";

        clear_idxcache();
        convert_fasta_file(input_filename, output_filename);

        test_sfasta("test_data/test_sfasta_fn.sfasta".to_string());
    }

    #[test]
    pub fn test_index() {
        let input_filename = "test_data/test_multiple.fna";
        let output_filename = "test_data/test_index.sfasta";

        clear_idxcache();
        convert_fasta_file(input_filename, output_filename);

        let (mut reader, idx) = open_file(output_filename);

        let idx = idx.unwrap();
        let i: Vec<&u64> = idx.1.iter().skip(2).take(1).collect();

        reader
            .seek(SeekFrom::Start(*i[0]))
            .expect("Unable to work with seek API");
        match bincode::deserialize_from::<_, EntryCompressedHeader>(&mut reader) {
            Ok(x) => x,
            Err(_) => panic!("Unable to read indexed SFASTA after jumping"),
        };
    }

    #[test]
    pub fn test_random_mode() {
        let input_filename = "test_data/test_multiple.fna";
        let output_filename = "test_data/test_random.sfasta";

        clear_idxcache();
        convert_fasta_file(input_filename, output_filename);

        println!("Converted...");

        let mut sequences = Sequences::new(output_filename);
        println!("Got sequences...");
        sequences.set_mode(SeqMode::Random);
        println!("Set Mode");
        let q = sequences.next().unwrap();
        println!("Got Next Seq");
        println!("Seq Len: {}", q.end);
        assert!(q.end > 0);
    }

    #[test]
    pub fn test_gen_sfasta_manually_and_read() {
        let output_filename = "test_bincode.sfasta";

        let out_file = File::create(output_filename.clone()).expect("Unable to write to file");
        let mut out_fh = BufWriter::new(out_file);

        let header = Header {
            citation: None,
            comment: None,
            id: Some(output_filename.to_string()),
            compression_type: CompressionType::LZ4,
        };

        bincode::serialize_into(&mut out_fh, &header).expect("Unable to write to bincode output");

        let entryheader = EntryCompressedHeader {
            id: "Example".to_string(),
            compression_type: CompressionType::LZ4,
            block_count: 2,
            comment: None,
            len: 100,
        };

        bincode::serialize_into(&mut out_fh, &entryheader)
            .expect("Unable to write EntryCompressedHeader");

        let entry = EntryCompressedBlock {
            id: "Example".to_string(),
            block_id: 0,
            compressed_seq: b"AAAAAAAAAAA".to_vec(),
        };

        bincode::serialize_into(&mut out_fh, &entry).expect("Unable to write first block");

        let entry = EntryCompressedBlock {
            id: "Example".to_string(),
            block_id: 1,
            compressed_seq: b"AAAAAAAAAAA".to_vec(),
        };

        bincode::serialize_into(&mut out_fh, &entry).expect("Unable to write first block");

        drop(out_fh);

        let file = match File::open(&output_filename) {
            Err(why) => panic!("Couldn't open: {}", why.to_string()),
            Ok(file) => file,
        };

        let mut reader = BufReader::with_capacity(512 * 1024, file);

        let header: Header = match bincode::deserialize_from(&mut reader) {
            Ok(x) => x,
            Err(_) => panic!("Header missing or malformed in SFASTA file"),
        };

        let entry: EntryCompressedHeader = match bincode::deserialize_from(&mut reader) {
            Ok(x) => x,
            Err(x) => panic!("Found error: {}", x),
        };

        for _i in 0..entry.block_count as usize {
            let _x: EntryCompressedBlock = bincode::deserialize_from(&mut reader).unwrap();
        }

        println!("{:#?}", header);

        test_sfasta(output_filename.to_string());
    }
}
