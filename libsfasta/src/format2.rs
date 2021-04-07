#![feature(const_generics)]

use std::borrow::Cow;
use std::convert::TryInto;
use std::hash::Hasher;

use lz4_flex::{compress_prepend_size, decompress_size_prepended};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_big_array::BigArray;
use twox_hash::xxh3::Hash64;

use crate::structs::CompressionType;

// big_array! { BigArray; }
// big_array! { BigArrayu16; }
#[derive(PartialEq, Serialize, Deserialize, Debug, Clone)]
struct Header<'header> {
    pub id: Option<&'header str>,
    pub comment: Option<&'header str>,
    pub citation: Option<&'header str>,
    pub compression_type: CompressionType,
    pub seq_start: u64,
    // TODO: Add in something so we can easily process files that are:
    // 1 single FASTA entry
    // < 512 FASTA entries...
    // > 512 FASTA entries...
}

#[derive(PartialEq, Serialize, Deserialize, Debug, Clone)]
struct IndexIndex {
    pub hashes: Vec<(u64, u64)>,
    pub idxid: Vec<u64>,
}

pub const BLOCK_SIZE: usize = 512;
// Should be blocks of 256 or 512 (bitpacking length, so it's convenient, 512 is doubling it)
#[derive(PartialEq, Serialize, Deserialize, Debug, Clone)]
struct Index {
    #[serde(with = "BigArray")]
    pub hashes: [u64; 512], // u64 hash of ID
    #[serde(with = "BigArray")]
    pub blocks: [u16; 512], // u16 of block ID // ELIGIBLE FOR BITPACKING
    #[serde(with = "BigArray")]
    pub locs: [u32; 512], // u32 offset // ELIGIBLE FOR BITPACKING
}

// TODO: Need entry, can't go directly to sequence blocks here...
// Large sequences may span multiple blocks, and need offsets and such...
impl Index {
    pub fn new(mut hashes: Vec<u64>, mut blocks: Vec<u16>, mut locs: Vec<u32>) -> Index {
        let hasheslen = hashes.len();
        let blockslen = hashes.len();
        let locslen = locs.len();
        assert!(hasheslen <= BLOCK_SIZE);
        assert!(blockslen <= BLOCK_SIZE);
        assert!(locslen <= BLOCK_SIZE);
        assert!(hasheslen == blockslen);
        assert!(hasheslen == locslen);

        if hasheslen < BLOCK_SIZE {
            // Pad everything to BLOCK_SIZE
            for _ in 0..BLOCK_SIZE - hasheslen {
                hashes.push(0);
                blocks.push(0);
                locs.push(0);
            }
        }

        assert!(hashes.len() == BLOCK_SIZE);
        assert!(blocks.len() == BLOCK_SIZE);
        assert!(locs.len() == BLOCK_SIZE);

        Index {
            hashes: hashes[0..512].try_into().unwrap(),
            blocks: blocks[0..512].try_into().unwrap(),
            locs: locs[0..512].try_into().unwrap(),
        }
    }
}

pub const SEQ_BLOCK_LEN: usize = 8 * 1024 * 1024;

#[derive(Deserialize, Serialize)]
enum SequenceType {
    DNA,
    RNA,
    Protein,
    Other,
}

type Location = (u32, u32);

/*
struct SequenceBlocker<'sequenceblocker> {
    bytes_buffer: Vec<u8>,
    cur_block: u64,
}

impl<'sequenceblocker> SequenceBlocker<'sequenceblocker> {
    pub fn new() -> SequenceBlocker<'sequenceblocker> {
        SequenceBlocker {
            bytes_buffer: Vec::with_capacity(SEQ_BLOCK_LEN),
            cur_block: 0,
        }
    }

    pub fn add_sequence(&mut self, seq: &[u8]) -> Option<bool> {
        let start = self.bytes_buffer.len();
        let end;
        if self.bytes_buffer.len() + seq.len() <= SEQ_BLOCK_LEN {
            self.bytes_buffer.extend_from_slice(seq);
            end = self.bytes_buffer.len();
        } else {
            
        }
    }
} */

#[derive(Deserialize, Serialize)]
struct Sequence<'sequence> {
    pub id: &'sequence str,
    pub seq_type: SequenceType,
    pub blocks: Vec<u32>,
    pub locations: Vec<Location>,
}

/*
#[derive(Deserialize, Serialize)]
struct SequenceBlock<'seqblock> {
    pub block_id: u64,
    #[serde(with = "serde_bytes")]
    pub bytes: Vec<u8>,
}

impl<'seqblock> SequenceBlock<'seqblock> {
    pub fn new(id: usize, bytes_buffer: Vec<u8>) -> SequenceBlock<'seqblock> {
        SequenceBlock {
            block_id: id as u64,
            bytes: &bytes_buffer,
        }
    }
} */

// Utility functions...
#[inline(always)]
pub fn hasher() -> Hash64 {
    Hash64::with_seed(129516)
}

#[inline(always)]
pub fn just_hash(x: &[u8]) -> u64 {
    let mut hasher = Hash64::with_seed(129516);
    hasher.write(x);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::Hasher;

    #[test]
    pub fn test_hasher() {
        let a: Vec<&str> = vec![
            "Chr1",
            "Chr2",
            "scaffold_902",
            "scaffold_1021",
            "scaffold_21349",
            "Chr9",
        ];
        let mut hasher = hasher();
        hasher.write(a[0].as_bytes());
        let hasher_val = hasher.finish();
        let hasher2_val = just_hash(a[0].as_bytes());
        assert_eq!(hasher_val, hasher2_val);
    }

    #[test]
    pub fn test_index_new() {
        let a: Vec<&str> = vec![
            "Chr1",
            "Chr2",
            "scaffold_902",
            "scaffold_1021",
            "scaffold_21349",
            "Chr9",
        ];

        let mut hashes: Vec<u64> = Vec::new();
        hashes.push(just_hash(a[0].as_bytes()));
        hashes.push(just_hash(a[1].as_bytes()));
        let mut blocks: Vec<u16> = Vec::new();
        blocks.push(0);
        blocks.push(1);
        let mut locs: Vec<u32> = Vec::new();
        locs.push(0);
        locs.push(10232321);

        let idx = Index::new(hashes, blocks, locs);
        assert!(idx.hashes.len() == BLOCK_SIZE);
    }
}
