#![feature(is_sorted)]
#![feature(trait_alias)]

pub mod build;
pub mod conversion;
pub mod disk;
pub mod fractal;
pub mod read;

pub use build::*;
pub use conversion::*;
pub use disk::*;
pub use fractal::*;
pub use read::*;

use bincode::{Decode, Encode};

pub trait Key = 'static
    + PartialOrd
    + PartialEq
    + Ord
    + Eq
    + std::fmt::Debug
    + Clone
    + Copy
    + Default
    + Encode
    + Decode;

pub trait Value = 'static + std::fmt::Debug + Copy + Encode + Decode;

// Impl can't be a trait because need for buffer for Disk version and no need for Read
