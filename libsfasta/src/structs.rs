use std::any::Any;
use std::io::prelude::*;

use serde::{Deserialize, Serialize};

use crate::index::Hashes;
use crate::utils::Bitpacked;

// SuperTrait -- needed for pyO3
pub trait ReadAndSeek: Read + Seek {}
impl<T: Read + Seek> ReadAndSeek for T {}

pub trait ReadAndSeekAndSend: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadAndSeekAndSend for T {}

pub trait WriteAndSeek: Write + Seek + Send + Sync {}
impl<T: Write + Seek + Send + Sync + Any> WriteAndSeek for T {}

pub trait T: Any {}
impl T for dyn WriteAndSeek {}

#[derive(PartialEq)]
pub enum SeqMode {
    Linear,
    Random,
}

#[derive(PartialEq, Serialize, Deserialize, Debug, Clone, Copy)]
pub enum CompressionType {
    ZSTD,   // 9 should be default compression ratio
    LZ4,    // 9 should be default compression ratio
    SNAPPY, // Not yet implemented -- IMPLEMENT
    GZIP,   // Please don't use this -- IMPLEMENT
    NAF,    // Not yet supported -- IMPLEMENT
    NONE,   // No Compression -- IMPLEMENT
    XZ,     // Implemented, 6 is default ratio
    BROTLI, // Implemented, 6 is default
}

impl Default for CompressionType {
    fn default() -> CompressionType {
        CompressionType::ZSTD
    }
}

pub const fn default_compression_level(ct: CompressionType) -> i32 {
    match ct {
        CompressionType::ZSTD => -3, // 19,
        CompressionType::LZ4 => 9,
        CompressionType::XZ => 6,
        CompressionType::BROTLI => 9,
        _ => 3,
    }
}

#[derive(PartialEq, Serialize, Deserialize, Debug, Clone)]
pub struct Header {
    pub id: Option<String>,
    pub comment: Option<String>,
    pub citation: Option<String>,
    pub compression_type: CompressionType,
}

/*

#[derive(Serialize, Deserialize)]
pub struct StoredIndexPlan {
    pub parts: u16,
    pub index: Vec<(u64, u64)>,
    pub min_size: u32,
    pub hash_type: Hashes,
    pub chunk_size: u32,
    pub index_len: u32,
}

const MINIMUM_CHUNK_SIZE: u32 = 8 * 1024 * 1024;

impl StoredIndexPlan {
    pub fn plan_from_parts<'a>(
        hashes: &'a [u64],
        _locs: &[Bitpacked],
        hash_type: Hashes,
        min_size: u32,
    ) -> (StoredIndexPlan, Vec<&'a [u64]>) {
        assert!(
            hashes[..].is_sorted(),
            "Hashes Vector must be sorted. Did you forget to finalize the index?"
        );

        assert!(hashes.len() <= u64::MAX as usize, "Hashes Vector must be smaller than 2^64... Contact Joseph to Discuss options or split into multiple files...");

        let hashes_count = hashes.len();
        let mut parts = 64;
        let mut chunk_size = (hashes_count as f64 / parts as f64).ceil() as u32;

        if hashes.len() < MINIMUM_CHUNK_SIZE as usize {
            parts = 1;
            chunk_size = MINIMUM_CHUNK_SIZE;
        } else {
            while chunk_size < MINIMUM_CHUNK_SIZE {
                parts -= 1;
                chunk_size = (hashes_count as f64 / parts as f64).ceil() as u32;
                if parts == 1 {
                    chunk_size = MINIMUM_CHUNK_SIZE;
                    break;
                }
            }
        }

        let mut index = Vec::new();
        let mut hash_splits = Vec::new();

        for i in 0..parts {
            let start = i as usize * chunk_size as usize;
            let mut end = (i as usize + 1) * chunk_size as usize;
            end = std::cmp::min(end, hashes.len());

            index.push((hashes[start], 0)); // 0 here is a placeholder!
            hash_splits.push(&hashes[start..end]);
        }

        // TODO: Split the Hashes
        // Bitpacked locs are already split into small chunks, so we can process that in the format.rs file

        (
            StoredIndexPlan {
                parts,
                index,
                min_size,
                hash_type,
                chunk_size,
                index_len: hashes.len() as u32,
            },
            hash_splits,
        )
    }
}
*/
