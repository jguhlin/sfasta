use std::any::Any;
use std::fs::{metadata, File};
use std::io::prelude::*;
use std::io::{BufWriter, SeekFrom};

use lz4::{Decoder, EncoderBuilder};
use rand::prelude::*;
use rand_chacha::ChaCha20Rng;
use serde::{Deserialize, Serialize};

use super::BLOCK_SIZE;
use crate::io;

// SuperTrait -- needed for pyO3
pub trait ReadAndSeek: Read + Seek {}
impl<T: Read + Seek> ReadAndSeek for T {}

pub trait WriteAndSeek: Write + Seek + Send + Sync {}
impl<T: Write + Seek + Send + Sync + Any> WriteAndSeek for T {}

pub trait T: Any {}
impl T for WriteAndSeek {}

#[derive(PartialEq)]
pub enum SeqMode {
    Linear,
    Random,
}

#[derive(PartialEq, Serialize, Deserialize, Debug, Clone, Copy)]
pub enum CompressionType {
    ZSTD,   // 9 should be default compression ratio
    LZ4,    // 9 should be default compression ratio
    SNAPPY, // Not yet implemented -- IMPLEMENT
    GZIP,   // Please don't use this -- IMPLEMENT
    NAF,    // Not yet supported -- IMPLEMENT
    NONE,   // No Compression -- IMPLEMENT
}

impl Default for CompressionType {
    fn default() -> CompressionType {
        return CompressionType::ZSTD;
    }
}

pub const fn default_compression_level(ct: CompressionType) -> i32 {
    match ct {
        CompressionType::ZSTD => 9, // 19,
        CompressionType::LZ4 => 9,
        _ => 3,
    }
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
        let (mut reader, idx) = io::open_file(filename);
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
