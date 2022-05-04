use std::fs::{metadata, File};
use std::io::{BufReader, Read};

use serde::{Deserialize, Serialize};

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

impl Iterator for Sequences {
    type Item = Sequence;

    fn next(&mut self) -> Option<Sequence> {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        let seq: Sequence =
            match bincode::serde::decode_from_std_read(&mut self.reader.as_mut(), bincode_config) {
                Ok(x) => x,
                Err(_) => {
                    println!("SeqStop");
                    return None;
                }
            };
        Some(seq)
    }
}
/*
impl Sequences {
    pub fn new(filename: String) -> Sequences {
        Sequences {
            reader: open_file(filename),
        }
    }
} */

#[inline]
pub fn generic_open_file(filename: &str) -> (usize, bool, Box<dyn Read + Send>) {
    let filesize = metadata(filename)
        .unwrap_or_else(|_| panic!("{}", &format!("Unable to open file: {}", filename)))
        .len();

    let file = match File::open(filename) {
        Err(why) => panic!("Couldn't open {}: {}", filename, why.to_string()),
        Ok(file) => file,
    };

    let file = BufReader::new(file);
    let mut compressed: bool = false;

    let fasta: Box<dyn Read + Send> = if filename.ends_with("gz") {
        compressed = true;
        Box::new(flate2::read::GzDecoder::new(file))
    } else if filename.ends_with("snappy") || filename.ends_with("sz") || filename.ends_with("sfai")
    {
        compressed = true;
        Box::new(snap::read::FrameDecoder::new(file))
    } else {
        Box::new(file)
    };

    (filesize as usize, compressed, fasta)
}

#[cfg(test)]
mod tests {
    use super::*;
}
