use std::fs::File;
use std::io::{BufReader, Read};

use serde::{Deserialize, Serialize};

use crate::io;

pub struct Sequences {
    reader: Box<dyn Read + Send>,
}

#[derive(PartialEq, Serialize, Deserialize, Clone, Debug)]
pub struct Sequence {
    pub seq: Vec<u8>,
    pub id: String,
    pub location: usize,
    pub end: usize,
}

// TODO: This is the right place to do this, but I feel it's happening somewhere
// else and wasting CPU cycles...
impl Sequence {
    pub fn make_uppercase(&mut self) {
        self.seq.make_ascii_uppercase();
    }
}

fn open_file(filename: String) -> Box<dyn Read + Send> {
    let file = match File::open(&filename) {
        Err(why) => panic!("Couldn't open {}: {}", filename, why.to_string()),
        Ok(file) => file,
    };

    let file = BufReader::with_capacity(64 * 1024 * 1024, file);

    let fasta: Box<dyn Read + Send> = if filename.ends_with("gz") {
        Box::new(flate2::read::GzDecoder::new(file))
    } else if filename.ends_with("snappy") || filename.ends_with("sz") {
        Box::new(snap::read::FrameDecoder::new(file))
    } else {
        Box::new(file)
    };

    let reader = BufReader::with_capacity(32 * 1024 * 1024, fasta);
    Box::new(reader)
}

impl Iterator for Sequences {
    type Item = Sequence;

    fn next(&mut self) -> Option<Sequence> {
        let seq: Sequence = match bincode::deserialize_from(&mut self.reader) {
            Ok(x) => x,
            Err(_) => {
                println!("SeqStop");
                return None;
            }
        };
        Some(seq)
    }
}

impl Sequences {
    pub fn new(filename: String) -> Sequences {
        Sequences {
            reader: open_file(filename),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sfasta;

    #[test]
    pub fn test_iosequences() {
        sfasta::clear_idxcache();
        sfasta::convert_fasta_file("test_data/test.fna", "test_data/test_sequences.sfasta");
        let mut sequences = Box::new(sfasta::Sequences::new("test_data/test_sequences.sfasta"));
        let sequence = sequences.next().unwrap();
        println!("{:#?}", sequence);
        assert!(sequence.id == "test");
        assert!(sequence.end == 670);
        assert!(sequence.location == 0);
        assert!(sequence.seq.len() == 670);
    }
    /*
    #[test]
    pub fn test_regular_fasta_file() {
        let mut sequences = Sequences::new("test_data/test.fna".to_string());
        let sequence = sequences.next().unwrap();
        assert!(sequence.id == "test");
        assert!(sequence.end == 669);
        assert!(sequence.location == 0);
        assert!(sequence.seq.len() == 669);




    }*/

    #[test]
    #[should_panic]
    pub fn test_empty() {
        let sequences = Box::new(sfasta::Sequences::new("test_data/empty.sfasta"));
        Box::new(io::SequenceSplitter3N::new(sequences));
    }

    #[test]
    pub fn test_sequences_impl() {
        sfasta::clear_idxcache();
        sfasta::convert_fasta_file(
            "test_data/test_multiple.fna",
            "test_data/test_sequences_impl.sfasta",
        );
        let seqs = Box::new(sfasta::Sequences::new(
            "test_data/test_sequences_impl.sfasta",
        ));
        let count = seqs.count();
        println!("Sequences Impl Count {}", count);
        assert!(count == 8);
    }
}
