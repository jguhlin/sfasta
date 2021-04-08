extern crate bincode;
extern crate bumpalo;
extern crate crossbeam;
extern crate flate2;
extern crate lz4;
extern crate rand;
extern crate serde;
extern crate serde_bytes;
extern crate snap;
extern crate thincollections;
extern crate zstd;

#[macro_use]
extern crate serde_big_array;

mod conversion;
mod directory;
mod fasta;
mod format;
mod io;
mod metadata;
mod parameters;
pub mod prelude;
mod sequence_block;
mod sequence_buffer;
mod structs;
mod utils;

pub use crate::fasta::*;
pub use crate::io::*;
pub use crate::structs::*;
pub use crate::utils::*;

use serde::{Deserialize, Serialize};

use std::sync::{Arc, RwLock};

pub const BLOCK_SIZE: usize = 8 * 1024 * 1024;

// TODO: Set a const for BufReader buffer size
//       Make it a global const, but also maybe make it configurable?
//       Reason being that network FS will benefit from larger buffers
// TODO: Also make BufWriter bufsize global, but ok to leave larger.

// sfasta is:
// bincode encoded
//   fasta ID
//   zstd compressed sequence
