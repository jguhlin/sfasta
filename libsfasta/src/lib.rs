#![feature(allocator_api)]
// #![feature(byte_slice_trim_ascii)]

pub mod conversion;
pub mod datastructures;
pub mod datatypes;
mod formats;
pub mod io;
pub mod parser;
pub mod utils;

pub mod prelude;

pub use crate::{datatypes::structs::*, io::*, utils::*};

pub const BINCODE_CONFIG: bincode::config::Configuration =
    bincode::config::standard().with_variable_int_encoding();
