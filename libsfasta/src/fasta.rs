use std::convert::TryInto;
use std::io::prelude::*;
use std::io::{BufRead, BufReader};

use crate::bytelines::ByteLinesReader;

use crate::io::generic_open_file;
use crate::structs::ReadAndSeek;

#[derive(Debug)]
pub struct Sequence {
    pub seq: Vec<u8>,
    pub id: String,
}

pub struct Fasta<R> {
    reader: R,
    buffer: Vec<u8>,
    seqbuffer: Vec<u8>,
    next_seqid: Option<String>,
    seqlen: usize,
}

impl<R: BufRead> Fasta<R> {
    pub fn from_buffer(mut in_buf: R) -> Fasta<R> {
        // let reader = BufReader::with_capacity(512 * 1024, in_buf);

        Fasta {
            reader: in_buf,
            buffer: Vec::with_capacity(1024),
            seqbuffer: Vec::with_capacity(32 * 1024 * 1024),
            next_seqid: None,
            seqlen: 0,
        }
    }

    /*
    pub fn from_file(filename: &str) -> Fasta<R> {
        let (_filesize, _, fh) = generic_open_file(filename);
        let reader = Box::new(BufReader::with_capacity(512 * 1024, fh));

        Fasta {
            reader,
            buffer: Vec::with_capacity(1024),
            seqbuffer: Vec::with_capacity(32 * 1024 * 1024),
            next_seqid: None,
            seqlen: 0,
        }
    } */
}

impl<R: BufRead> Iterator for Fasta<R> {
    type Item = Sequence;

    fn next(&mut self) -> Option<Sequence> {
        while let Ok(bytes_read) = self.reader.read_until(b'\n', &mut self.buffer) {
            if bytes_read == 0 {
                if self.seqlen > 0 {
                    // println!("{:#?}",
                    // std::str::from_utf8(&self.seqbuffer[..self.seqlen]).unwrap());
                    let seq = Sequence {
                        seq: self.seqbuffer[..self.seqlen].to_vec(),
                        id: self.next_seqid.clone().unwrap(),
                    };
                    self.seqlen = 0;
                    return Some(seq);
                } else {
                    return None;
                }
            } else {
                match self.buffer[0] {
                    // 62 is a > meaning we have a new sequence id.
                    62 => {
                        let slice_end = if self.buffer[bytes_read - 1] == b'\n' {
                            bytes_read.saturating_sub(1)
                        } else {
                            bytes_read
                        };
                        //                        let slice_end = bytes_read; //.saturating_sub(1);
                        let next_id = String::from_utf8(self.buffer[1..slice_end].to_vec())
                            .expect("Invalid UTF-8 encoding...");
                        self.buffer.clear();
                        let next_id = next_id.split(' ').next().unwrap().trim().to_string();
                        let id = self.next_seqid.replace(next_id);

                        if self.seqlen > 0 {
                            assert!(id.is_some());
                            let seq = Sequence {
                                seq: self.seqbuffer[..self.seqlen].to_vec(),
                                id: id.unwrap(),
                            };
                            self.seqbuffer.clear();
                            self.seqlen = 0;
                            return Some(seq);
                        }
                    }
                    _ => {
                        let slice_end = if self.buffer[bytes_read - 1] == b'\n' {
                            bytes_read.saturating_sub(1)
                        } else {
                            bytes_read
                        };
                        // let slice_end = bytes_read; //.saturating_sub(1);
                        self.seqbuffer.extend_from_slice(&self.buffer[0..slice_end]);
                        self.seqlen = self.seqlen.saturating_add(slice_end);
                        self.buffer.clear();
                    }
                }
            }
        }
        unreachable!();
    }
}

pub fn summarize_fasta(fasta_buf: &mut dyn BufRead) -> (usize, Vec<String>, Vec<usize>) {
    let mut entries: usize = 0;
    let mut ids: Vec<String> = Vec::with_capacity(2 * 1024 * 1024);
    let mut lengths: Vec<usize> = Vec::with_capacity(2 * 1024 * 1024);
    let mut length: usize = 0;

    let mut lines = fasta_buf.byte_lines();
    let mut first = true;
    while let Some(line) = lines.next() {
        let line = line.expect("Error parsing FASTA file");
        if line.starts_with(b">") {
            let id = std::str::from_utf8(&line[1..]).expect("Unable to convert FASTA header to string");
            ids.push(id.to_string());
            
            if first {
                first = false;
            } else {
                lengths.push(length.clone());
            }
            
            entries += 1;
            length = 0;
        } else {
            length += line.len();
        }
    }
    lengths.push(length);

    return (entries, ids, lengths);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;

    #[test]
    pub fn test_fasta_parse() {
        let fakefasta =
            b">Hello\nACTGCATCACTGACCTA\n>Second\nACTTGCAACTTGGGACACAACATGTA\n".to_vec();
        let fakefasta_ = fakefasta.as_slice();
        let mut fasta = Fasta::from_buffer(BufReader::new(fakefasta_));
        let j = fasta.next();
        let j = fasta.next();
    }

    #[test]
    pub fn test_summarize_fasta() {
        let fakefasta =
            b">Hello\nACTGCATCACTGACCTA\n>Second\nACTTGCAACTTGGGACACAACATGTA\n".to_vec();
        let fakefasta_ = fakefasta.as_slice();
        let mut buf = BufReader::new(fakefasta_);
        let j = summarize_fasta(&mut buf);
        assert!(j.0 == 2);
        println!("{:#?}", j);
        assert!(j.1[0] == "Hello");
        assert!(j.1[1] == "Second");
        assert!(j.2[0] == 17);
        assert!(j.2[1] == 26);
    }
}
