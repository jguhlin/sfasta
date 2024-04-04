pub use crate::conversion::Converter; // generic_open_file,
pub use crate::{
    convenience::*,
    datatypes::Sequence,
    formats::{
        fasta::*,
        fastq::*,
        sfasta::{SeqMode, Sequences, Sfasta, SfastaParser},
    },
    masking::*,
};
pub use libcompression::*;
