pub use crate::conversion::Converter; // generic_open_file,
pub use crate::{
    datatypes::{compression_profile::*, simple::Metadata, Sequence},
    formats::sfasta::{SeqMode, Sequences, Sfasta, SfastaParser},
};
pub use libcompression::*;
pub use crate::parser::std::{open_from_buffer, open_with_buffer};

#[cfg(feature = "async")]
pub use crate::parser::async_parser::open_from_file_async;
