pub use crate::conversion::Converter; // generic_open_file,
pub use crate::{
    datatypes::{Sequence, compression_profile::*, simple::Metadata},
    formats::sfasta::{SeqMode, Sequences, Sfasta, SfastaParser},
    
};
pub use libcompression::*;
