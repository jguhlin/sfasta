// #![feature(byte_slice_trim_ascii)]

extern crate bincode;
extern crate crossbeam;
extern crate rayon;
#[macro_use]
extern crate itertools;

pub mod compression;
mod compression_stream_buffer;
mod conversion;
pub mod data_types;
pub mod dual_level_index;
mod formats;
mod io;
pub mod masking;
pub mod utils;
pub mod convenience;

pub mod prelude;

pub use crate::data_types::structs::*;
pub use crate::io::*;
pub use crate::utils::*;
