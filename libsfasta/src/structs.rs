use std::any::Any;
use std::io::prelude::*;

use serde::{Deserialize, Serialize};

// SuperTrait -- needed for pyO3
pub trait ReadAndSeek: Read + Seek {}
impl<T: Read + Seek> ReadAndSeek for T {}

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
}

impl Default for CompressionType {
    fn default() -> CompressionType {
        CompressionType::ZSTD
    }
}

pub const fn default_compression_level(ct: CompressionType) -> i32 {
    match ct {
        CompressionType::ZSTD => 9, // 19,
        CompressionType::LZ4 => 9,
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
