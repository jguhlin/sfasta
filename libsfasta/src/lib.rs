extern crate bincode;
extern crate bumpalo;
extern crate crossbeam;
extern crate flate2;
extern crate lz4;
extern crate serde;
extern crate serde_bytes;
extern crate rand;
extern crate snap;
extern crate thincollections;
extern crate zstd;

mod fasta;
mod utils;
mod structs;
mod conversions;
mod io;

use crate::fasta::*;
use crate::utils::*;
use crate::structs::*;

use serde::{Deserialize, Serialize};

use std::sync::{Arc, RwLock};

pub const BLOCK_SIZE: usize = 8 * 1024 * 1024;

// TODO: Set a const for BufReader buffer size
//       Make it a global const, but also maybe make it configurable?
//       Reason being that network FS will benefit from larger buffers
// TODO: Also make BufWriter bufsize global, but ok to leave larger.

// sfasta is:
// bincode encoded
//   fasta ID
//   zstd compressed sequence

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::SeekFrom;

    #[test]
    pub fn test_entry() {
        let seq = b"ACTGGACTACAGTTCAGGACATCACTTTCACTACTAGTGAGATTGACCACTA".to_vec();
        let oseq = seq.clone();
        let e = Entry {
            id: "Test".to_string(),
            len: seq.len() as u64,
            seq: seq,
            comment: None,
        };

        let ec = e.compress(CompressionType::LZ4, 9);
        //        let e = ec.decompress();

        //        assert!(e.seq == oseq);
    }

    #[test]
    pub fn convert_fasta_to_sfasta_and_index() {
        let input_filename = "test_data/test_multiple.fna";
        let output_filename = "test_data/test_sfasta_convert_and_index.sfasta";

        clear_idxcache();
        convert_fasta_file(input_filename, output_filename);

        let idx_filename = index(output_filename);
        assert!(idx_filename == "test_data/test_sfasta_convert_and_index.sfai");

        load_index(output_filename);
    }

    #[test]
    pub fn test_sfasta_fn() {
        let input_filename = "test_data/test_large_multiple.fna";
        let output_filename = "test_data/test_sfasta_fn.sfasta";

        clear_idxcache();
        convert_fasta_file(input_filename, output_filename);

        test_sfasta("test_data/test_sfasta_fn.sfasta".to_string());
    }

    #[test]
    pub fn test_index() {
        let input_filename = "test_data/test_multiple.fna";
        let output_filename = "test_data/test_index.sfasta";

        clear_idxcache();
        convert_fasta_file(input_filename, output_filename);

        let (mut reader, idx) = open_file(output_filename);

        let idx = idx.unwrap();
        let i: Vec<&u64> = idx.1.iter().skip(2).take(1).collect();

        reader
            .seek(SeekFrom::Start(*i[0]))
            .expect("Unable to work with seek API");
        match bincode::deserialize_from::<_, EntryCompressedHeader>(&mut reader) {
            Ok(x) => x,
            Err(_) => panic!("Unable to read indexed SFASTA after jumping"),
        };
    }

    #[test]
    pub fn test_random_mode() {
        let input_filename = "test_data/test_multiple.fna";
        let output_filename = "test_data/test_random.sfasta";

        clear_idxcache();
        convert_fasta_file(input_filename, output_filename);

        println!("Converted...");

        let mut sequences = Sequences::new(output_filename);
        println!("Got sequences...");
        sequences.set_mode(SeqMode::Random);
        println!("Set Mode");
        let q = sequences.next().unwrap();
        println!("Got Next Seq");
        println!("Seq Len: {}", q.end);
        assert!(q.end > 0);
    }

    #[test]
    pub fn test_gen_sfasta_manually_and_read() {
        let output_filename = "test_bincode.sfasta";

        let out_file = File::create(output_filename.clone()).expect("Unable to write to file");
        let mut out_fh = BufWriter::new(out_file);

        let header = Header {
            citation: None,
            comment: None,
            id: Some(output_filename.to_string()),
            compression_type: CompressionType::LZ4,
        };

        bincode::serialize_into(&mut out_fh, &header).expect("Unable to write to bincode output");

        let entryheader = EntryCompressedHeader {
            id: "Example".to_string(),
            compression_type: CompressionType::LZ4,
            block_count: 2,
            comment: None,
            len: 100,
        };

        bincode::serialize_into(&mut out_fh, &entryheader)
            .expect("Unable to write EntryCompressedHeader");

        let entry = EntryCompressedBlock {
            id: "Example".to_string(),
            block_id: 0,
            compressed_seq: b"AAAAAAAAAAA".to_vec(),
        };

        bincode::serialize_into(&mut out_fh, &entry).expect("Unable to write first block");

        let entry = EntryCompressedBlock {
            id: "Example".to_string(),
            block_id: 1,
            compressed_seq: b"AAAAAAAAAAA".to_vec(),
        };

        bincode::serialize_into(&mut out_fh, &entry).expect("Unable to write first block");

        drop(out_fh);

        let file = match File::open(&output_filename) {
            Err(why) => panic!("Couldn't open: {}", why.to_string()),
            Ok(file) => file,
        };

        let mut reader = BufReader::with_capacity(512 * 1024, file);

        let header: Header = match bincode::deserialize_from(&mut reader) {
            Ok(x) => x,
            Err(_) => panic!("Header missing or malformed in SFASTA file"),
        };

        let entry: EntryCompressedHeader = match bincode::deserialize_from(&mut reader) {
            Ok(x) => x,
            Err(x) => panic!("Found error: {}", x),
        };

        for _i in 0..entry.block_count as usize {
            let _x: EntryCompressedBlock = bincode::deserialize_from(&mut reader).unwrap();
        }

        println!("{:#?}", header);

        test_sfasta(output_filename.to_string());
    }
}
