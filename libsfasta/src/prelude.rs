pub use crate::conversion::Converter; // generic_open_file,
pub use crate::{
    datatypes::{compression_profile::*, simple::Metadata, Sequence},
    formats::sfasta::{SeqMode, Sfasta},
};
pub use libcompression::*;

#[cfg(feature = "async")]
pub use crate::parser::async_parser::open_from_file_async;

#[cfg(not(feature = "async"))]
pub use crate::parser::std::open_with_buffer;
