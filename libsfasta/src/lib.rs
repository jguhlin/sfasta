// #![feature(byte_slice_trim_ascii)]

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

pub const BINCODE_CONFIG: bincode::config::Configuration = bincode::config::standard().with_variable_int_encoding();
