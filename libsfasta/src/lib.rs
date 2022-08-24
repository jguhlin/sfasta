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
pub mod dual_level_index;
mod fasta;
mod fastq;
mod format;
// pub mod index;
pub mod data_types;
mod index_directory;
mod io;
mod metadata;
mod parameters;
pub mod prelude;
mod sequence_block;
pub mod structs;
mod utils;

pub use crate::fasta::*;
pub use crate::io::*;
pub use crate::structs::*;
pub use crate::utils::*;
