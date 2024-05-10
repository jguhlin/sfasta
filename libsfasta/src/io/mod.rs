//! # IO
//! This module contains the IO functions for the library. The goal is
//! to not write directly to files but to buffers, to allow for more
//! flexible implementations as needed.

pub mod worker;

pub use worker::*;
