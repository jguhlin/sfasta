use bytelines::ByteLines;
use bytelines::ByteLinesReader;
use simdutf8::basic::from_utf8;

use std::io::BufRead;

#[derive(Debug)]
pub struct Sequence {
    pub seq: Vec<u8>,
    pub id: String,
    pub header: String,
    pub scores: Vec<u8>,
}

pub struct Fastq<R: BufRead> {
    reader: ByteLines<R>,
}

impl<R: BufRead> Fastq<R> {
    pub fn _from_buffer(in_buf: R) -> Fastq<R> {
        Fastq {
            reader: in_buf.byte_lines(),
        }
    }
}

impl<R: BufRead> Iterator for Fastq<R> {
    type Item = Sequence;

    fn next(&mut self) -> Option<Sequence> {
        let id;
        let seq;
        let scores;

        if let Some(line) = self.reader.next() {
            id = from_utf8(&line.expect("Unable to read FASTQ line")[1..])
                .expect("Unable to parse FASTQ ID")
                .to_string();
        } else {
            return None;
        }

        if let Some(line) = self.reader.next() {
            seq = line.expect("Unable to read FASTQ line").to_vec();
        } else {
            panic!("Malformed/truncated FASTQ file");
        }

        self.reader.next().expect("Malformed/truncated FASTQ file").expect("Unable to read FASTQ line");

        if let Some(line) = self.reader.next() {
            scores = line.expect("Unable to read FASTQ line").to_vec();
        } else {
            panic!("Malformed/truncated FASTQ file");
        }

        let split: Vec<&str> = id.splitn(2, ' ').collect();
        let id = split[0].trim().to_string();
        let header = if split.len() == 2 {
            split[1].trim().to_string()
        } else {
            "".to_string()
        };

        Some(Sequence {
            id,
            seq: seq.to_vec(),
            scores: scores.to_vec(),
            header,
        })
    }
}

#[cfg(test)]
mod tests {
    

}
