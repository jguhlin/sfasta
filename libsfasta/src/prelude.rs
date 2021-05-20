pub use crate::structs::{Entry, EntryCompressedBlock, EntryCompressedHeader, SeqMode, Sequences};

pub use crate::io::{create_index, open_file};

pub use crate::conversion::{convert_fasta, generic_open_file};
pub use crate::fasta::{count_fasta_entries, summarize_fasta};
pub use crate::format::{Sfasta, SfastaParser};
pub use crate::index::IDIndexer;

/*pub use crate::fasta::*;
pub use crate::utils::*;
pub use crate::structs::*;
pub use crate::io::*;*/
