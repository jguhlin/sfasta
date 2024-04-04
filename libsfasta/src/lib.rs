// #![feature(byte_slice_trim_ascii)]

extern crate bincode;
extern crate crossbeam;
extern crate rayon;
#[macro_use]
extern crate itertools;

pub mod block_index;
pub mod convenience;
pub mod conversion;
pub mod datastructures;
pub mod datatypes;
mod formats;
pub mod io;
pub mod masking;
pub mod utils;

pub mod prelude;

pub use crate::{datatypes::structs::*, io::*, utils::*};
