// TODO: Move FASTA handling code here...
// To make uniform...

use crate::utils::generic_open_file;
use std::convert::TryInto;
use std::io::{BufRead, BufReader};

pub struct Sequence {
    pub seq: Vec<u8>,
    pub id: String,
}

pub struct Fasta {
    reader: Box<dyn BufRead + Send>,
    buffer: Vec<u8>,
    seqbuffer: Vec<u8>,
    next_seqid: Option<String>,
    seqlen: usize,
    pub filesize: u64,
}

impl Fasta {
    pub fn new(filename: &str) -> Fasta {
        let (filesize, _, fh) = generic_open_file(filename);
        let reader = Box::new(BufReader::with_capacity(512 * 1024, fh));

        Fasta {
            reader,
            buffer: Vec::with_capacity(1024),
            seqbuffer: Vec::with_capacity(32 * 1024 * 1024),
            next_seqid: None,
            seqlen: 0,
            filesize: filesize.try_into().expect("Unable to convert to u64"),
        }
    }
}

impl Iterator for Fasta {
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

#[cfg(test)]
mod tests {
    use super::*;

    // TODO: Test compressed...
    #[test]
    pub fn test_fasta_iterator_multiple() {
        let mut fasta = Fasta::new("test_data/test_multiple.fna");
        let seq = fasta.next().unwrap();
        assert!(seq.id == "test1");
        assert!(seq.seq == b"ACTGACTGACTGACTGACTGNNNNNNNNNNNNNNNNNNNACTGACTGGGTACCATTAGACACTGACTGACTGACTGACTGNNNNNNNNNNNNNNNNNNNACTGACTGGGTACCATTAGACCATTATA");
        let seq_ids: Vec<String> = fasta.map(|x| x.id).collect();
        assert!(seq_ids.len() == 7);
    }

    #[test]
    pub fn test_fasta_iterator_empty() {
        let fasta = Fasta::new("test_data/empty.fna");
        let seq_ids: Vec<String> = fasta.map(|x| x.id).collect();
        assert!(seq_ids.is_empty());
    }

    #[test]
    pub fn test_fasta_iterator_single() {
        let fasta = Fasta::new("test_data/test.fna");
        let seq_ids: Vec<String> = fasta.map(|x| x.id).collect();
        assert!(seq_ids.len() == 1);
        let mut fasta = Fasta::new("test_data/test.fna");
        let x = fasta.next().unwrap();
        println!("SeqID: {}", x.id);
        println!("Seq: {}", std::str::from_utf8(&x.seq).unwrap());
        println!("SeqLen: {}", x.seq.len());
        assert!(x.seq.len() == 670);
    }
}
