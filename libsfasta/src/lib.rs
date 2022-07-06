#![feature(is_sorted)]

extern crate bincode;
extern crate bytelines;
extern crate crossbeam;
extern crate rand;
extern crate rayon;
#[macro_use]
extern crate itertools;

extern crate flate2;
extern crate lz4_flex;
extern crate snap;
extern crate zstd;

mod compression_stream_buffer;
mod conversion;
mod dict;
// mod directory;
mod fasta;
mod fastq;
mod format;
pub mod index;
mod index_directory;
mod io;
mod metadata;
mod parameters;
pub mod prelude;
mod seqloc_block;
mod sequence_block;
pub mod structs;
mod types;
mod utils;

pub use crate::fasta::*;
pub use crate::io::*;
pub use crate::structs::*;
pub use crate::utils::*;

pub const BLOCK_SIZE: usize = 8 * 1024 * 1024;

// TODO: Set a const for BufReader buffer size
//       Make it a global const, but also maybe make it configurable?
//       Reason being that network FS will benefit from larger buffers
// TODO: Also make BufWriter bufsize global, but ok to leave larger.
