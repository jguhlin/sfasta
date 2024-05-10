pub use crate::conversion::Converter; // generic_open_file,
pub use crate::{
    datatypes::{compression_profile::*, simple::Metadata, Sequence},
    formats::sfasta::{SeqMode, Sequences, Sfasta, SfastaParser},
};
pub use libcompression::*;
