use bytelines::ByteLines;
use bytelines::ByteLinesReader;
use simdutf8::basic::from_utf8;

use std::io::BufRead;

#[derive(Debug)]
pub struct Sequence {
    pub seq: Vec<u8>,
    pub id: String,
    pub plus: String,
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
        let plus;
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

        if let Some(line) = self.reader.next() {
            plus = from_utf8(&line.expect("Unable to read FASTQ line")[1..])
                .expect("Unable to parse Plus line")
                .to_string();
        } else {
            panic!("Malformed/truncated FASTQ file");
        }

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
            plus,
            scores: scores.to_vec(),
            header,
        })
    }
}

pub fn _summarize_fasta(fasta_buf: &mut dyn BufRead) -> (usize, Vec<String>, Vec<usize>) {
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

#[cfg(test)]
mod tests {

    /*#[test]
    pub fn test_fasta_parse() {
        let fakefasta =
            b">Hello\nACTGCATCACTGACCTA\n>Second\nACTTGCAACTTGGGACACAACATGTA\n".to_vec();
        let fakefasta_ = fakefasta.as_slice();
        let mut fasta = Fasta::from_buffer(BufReader::new(fakefasta_));
        let _j = fasta.next();
        let _j = fasta.next();
    }

    #[test]
    pub fn test_fasta_parse_rest_of_the_header() {
        let fakefasta =
            b">Hello I have more information in the rest of the FASTA header\nACTGCATCACTGACCTA\n>Second\nACTTGCAACTTGGGACACAACATGTA\n".to_vec();
        let fakefasta_ = fakefasta.as_slice();
        let mut fasta = Fasta::from_buffer(BufReader::new(fakefasta_));
        let s = fasta.next().unwrap();
        println!("{:#?}", s.header);
        assert!(&s.header == "I have more information in the rest of the FASTA header");
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
    } */
}
