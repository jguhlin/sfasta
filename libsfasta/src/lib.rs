#![feature(byte_slice_trim_ascii)]
#![feature(is_sorted)]

extern crate bincode;
extern crate crossbeam;
extern crate rayon;
#[macro_use]
extern crate itertools;

extern crate flate2;
extern crate lz4_flex;
extern crate zstd;

pub mod compression;
mod compression_stream_buffer;
mod conversion;
pub mod data_types;
pub mod dual_level_index;
mod fasta;
mod fastq;
mod format;
mod io;
pub mod masking;
mod utils;

pub mod prelude;

pub use crate::data_types::structs::*;
pub use crate::fasta::*;
pub use crate::io::*;
pub use crate::utils::*;
