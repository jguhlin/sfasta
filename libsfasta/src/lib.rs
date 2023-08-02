// #![feature(byte_slice_trim_ascii)]

extern crate bincode;
extern crate crossbeam;
extern crate rayon;
#[macro_use]
extern crate itertools;

pub mod compression;
pub mod convenience;
pub mod conversion;
pub mod datatypes;
pub mod dual_level_index;
mod formats;
pub mod io;
pub mod masking;
pub mod utils;

pub mod prelude;

pub use crate::datatypes::structs::*;
pub use crate::io::*;
pub use crate::utils::*;
