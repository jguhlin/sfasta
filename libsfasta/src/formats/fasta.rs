use bytelines::ByteLinesReader;
use simdutf8::basic::from_utf8;

use std::io::BufRead;

#[derive(Debug)]
pub struct Sequence {
    pub seq: Vec<u8>,
    pub id: String,
    pub header: Option<String>,
}

impl Sequence {
    pub fn into_parts(self) -> (String, Option<String>, Vec<u8>) {
        {
            (self.id, self.header, self.seq)
        }
    }
}

pub struct Fasta<'fasta, R> {
    reader: &'fasta mut R,
    buffer: Vec<u8>,
    seqbuffer: Vec<u8>,
    next_seqid: Option<String>,
    next_header: Option<String>,
    seqlen: usize,
}

impl<'fasta, R: BufRead> Fasta<'fasta, R> {
    pub fn from_buffer(in_buf: &mut R) -> Fasta<R> {
        Fasta {
            reader: in_buf,
            buffer: Vec::with_capacity(1024),
            seqbuffer: Vec::with_capacity(2048),
            next_seqid: None,
            next_header: None,
            seqlen: 0,
        }
    }
}

impl<'fasta, R: BufRead> Iterator for Fasta<'fasta, R> {
    type Item = Sequence;

    fn next(&mut self) -> Option<Sequence> {
        while let Ok(bytes_read) = self.reader.read_until(b'\n', &mut self.buffer) {
            if bytes_read == 0 {
                if self.seqlen > 0 {
                    let seq = Sequence {
                        seq: self.seqbuffer[..self.seqlen].to_vec(),
                        id: self.next_seqid.take().unwrap(),
                        header: self.next_header.take(),
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

                        let next_id = from_utf8(&self.buffer[1..slice_end])
                            .expect("Invalid UTF-8 encoding...")
                            .to_string();

                        let next_id = next_id.trim();

                        self.buffer.clear();
                        let split: Vec<&str> = next_id.splitn(2, ' ').collect();
                        let next_id = split[0].trim().to_string();
                        let mut header = if split.len() == 2 {
                            Some(split[1].trim().to_string())
                        } else {
                            None
                        };

                        let id = self.next_seqid.replace(next_id);
                        std::mem::swap(&mut header, &mut self.next_header);

                        if self.seqlen > 0 {
                            assert!(id.is_some());

                            // Use the last seqlen as the new buffer's size
                            let mut seqbuf = Vec::with_capacity(self.seqlen);
                            std::mem::swap(&mut self.seqbuffer, &mut seqbuf);
                            seqbuf.truncate(self.seqlen);

                            let seq = Sequence {
                                seq: seqbuf,
                                id: id.unwrap(),
                                header,
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
            let id = from_utf8(&line[1..]).expect("Unable to convert FASTA header to string");
            ids.push(id.to_string());

            if first {
                first = false;
            } else {
                lengths.push(length);
            }

            entries += 1;
            length = 0;
        } else {
            length += line.len();
        }
    }
    lengths.push(length);

    (entries, ids, lengths)
}

pub fn count_fasta_entries(fasta_buf: &mut dyn BufRead) -> usize {
    let mut entries: usize = 0;

    let mut lines = fasta_buf.byte_lines();
    while let Some(line) = lines.next() {
        let line = line.expect("Error parsing FASTA file");
        if line.starts_with(b">") {
            entries += 1;
        }
    }

    entries
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
        let inner = &mut BufReader::new(fakefasta_);
        let mut fasta = Fasta::from_buffer(inner);
        let _j = fasta.next();
        let _j = fasta.next();
    }

    #[test]
    pub fn test_fasta_parse_rest_of_the_header() {
        let fakefasta =
            b">Hello I have more information in the rest of the FASTA header\nACTGCATCACTGACCTA\n>Second\nACTTGCAACTTGGGACACAACATGTA\n".to_vec();
        let fakefasta_ = fakefasta.as_slice();
        let mut inner = BufReader::new(fakefasta_);
        let mut fasta = Fasta::from_buffer(&mut inner);
        let s = fasta.next().unwrap();
        println!("{:#?}", s.header);
        assert!(
            &s.header
                == &Some("I have more information in the rest of the FASTA header".to_string())
        );
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
