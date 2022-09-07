use simdutf8::basic::from_utf8;

use std::io::BufRead;

use crate::datatypes::Sequence;

pub struct Fastq<'fastq, R: BufRead> {
    reader: &'fastq mut R,
    buffer: Vec<u8>,
    seqbuffer: Vec<u8>,
    scores_buffer: Vec<u8>,
    seqid: Option<String>,
    header: Option<String>,
    seqlen: usize,
    state: State,
}

#[allow(dead_code)]
impl<'fastq, R: BufRead> Fastq<'fastq, R> {
    pub fn from_buffer(in_buf: &'fastq mut R) -> Fastq<'fastq, R> {
        Fastq {
            reader: in_buf,
            buffer: Vec::with_capacity(1024),
            seqbuffer: Vec::with_capacity(2048),
            scores_buffer: Vec::with_capacity(2048),
            seqid: None,
            header: None,
            seqlen: 0,
            state: State::ID,
        }
    }

    pub fn into_reader(self) -> &'fastq mut R {
        self.reader
    }
}

enum State {
    ID,
    Sequence,
    Plus,
    Scores,
}

// TODO: This does extra moving by using a generic buffer
// Place the "MATCH" before the while let.... (prob don't need while let anymore...)
impl<'a, R: BufRead> Iterator for Fastq<'a, R> {
    type Item = Sequence;

    fn next(&mut self) -> Option<Sequence> {
        loop {
            match self.state {
                State::ID => {
                    if let Ok(bytes_read) = self.reader.read_until(b'\n', &mut self.buffer) {
                        if bytes_read == 0 {
                            if self.seqlen > 0 {
                                let seq = Sequence {
                                    sequence: self.seqbuffer[..self.seqlen].to_vec(),
                                    id: self.seqid.take().unwrap(),
                                    header: self.header.take(),
                                    scores: Some(self.scores_buffer[..self.seqlen].to_vec()),
                                };
                                self.buffer.clear();
                                self.seqlen = 0;
                                return Some(seq);
                            } else {
                                return None;
                            }
                        } else if self.buffer[0] == b'@' {
                            let idline = from_utf8(&self.buffer[1..]).unwrap().to_string();
                            let idline = idline.trim();

                            let split: Vec<&str> = idline.splitn(2, ' ').collect();
                            self.seqid = Some(split[0].to_string());
                            if split.len() > 1 {
                                self.header = Some(split[1].to_string());
                            } else {
                                self.header = Some("".to_string());
                            }
                            self.buffer.clear();
                            self.state = State::Sequence;
                        } else {
                            panic!("Invalid FASTQ file");
                        }
                    }
                }
                State::Sequence => {
                    if let Ok(bytes_read) = self.reader.read_until(b'\n', &mut self.seqbuffer) {
                        if bytes_read == 0 {
                            panic!("Invalid FASTQ file");
                        } else {
                            let mut end = self.seqbuffer.len() - 1;
                            while self.seqbuffer[end].is_ascii_whitespace() {
                                end -= 1;
                            }
                            self.seqlen = end + 1;
                            self.state = State::Plus;
                        }
                    }
                }
                State::Plus => {
                    if let Ok(bytes_read) = self.reader.read_until(b'\n', &mut self.buffer) {
                        if bytes_read == 0 {
                            panic!("Invalid FASTQ file");
                        }
                        self.state = State::Scores;
                    }
                }
                State::Scores => {
                    if let Ok(bytes_read) = self.reader.read_until(b'\n', &mut self.scores_buffer) {
                        if bytes_read == 0 {
                            panic!("Invalid FASTQ file");
                        } else {
                            assert!(self.seqid.is_some());

                            let mut seqbuffer = Vec::with_capacity(self.seqlen);
                            let mut scores_buffer = Vec::with_capacity(self.seqlen);
                            std::mem::swap(&mut self.seqbuffer, &mut seqbuffer);
                            std::mem::swap(&mut self.scores_buffer, &mut scores_buffer);
                            seqbuffer.truncate(self.seqlen);
                            scores_buffer.truncate(self.seqlen);

                            let seq = Sequence {
                                sequence: seqbuffer,
                                id: self.seqid.take().unwrap(),
                                header: self.header.take(),
                                scores: Some(scores_buffer),
                            };
                            self.buffer.clear();
                            self.seqlen = 0;
                            self.state = State::ID;
                            return Some(seq);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_fastq() {
        let fastq_data = b"@seq1
ACGT
+
IIII
@seq2
ACGT
+
IIII
@seq3
ACGT
+
IIII
@seq4
ACGT
+
IIII
";
        let mut cursor = Cursor::new(fastq_data);
        let mut fastq = Fastq::from_buffer(&mut cursor);
        let seq = fastq.next().unwrap();
        assert_eq!(seq.id, "seq1");
        assert_eq!(seq.sequence, b"ACGT");
        assert_eq!(seq.scores, Some(b"IIII".to_vec()));
        let seq = fastq.next().unwrap();
        assert_eq!(seq.id, "seq2");
        assert_eq!(seq.sequence, b"ACGT");
        assert_eq!(seq.scores.unwrap(), b"IIII");
        let seq = fastq.next().unwrap();
        assert_eq!(seq.id, "seq3");
        assert_eq!(seq.sequence, b"ACGT");
        assert_eq!(seq.scores, Some(b"IIII".to_vec()));
        let seq = fastq.next().unwrap();
        assert_eq!(seq.id, "seq4");
        assert_eq!(seq.sequence, b"ACGT");
        assert_eq!(seq.scores, Some(b"IIII".to_vec()));
        assert!(fastq.next().is_none());
    }
}
