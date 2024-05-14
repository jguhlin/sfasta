#[cfg(feature = "async")]
pub mod async_parser;

#[cfg(feature = "async")]
pub use async_parser::*;

#[cfg(not(feature = "async"))]
pub mod std;

#[cfg(not(feature = "async"))]
pub use std::*;
