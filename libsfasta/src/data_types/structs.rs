use std::any::Any;
use std::io::prelude::*;

// SuperTrait -- needed for pyO3
pub trait ReadAndSeek: Read + Seek {}
impl<T: Read + Seek> ReadAndSeek for T {}

pub trait ReadAndSeekAndSend: Read + Seek {}
impl<T: Read + Seek> ReadAndSeekAndSend for T {}

pub trait WriteAndSeek: Write + Seek {}
impl<T: Write + Seek + Any> WriteAndSeek for T {}

pub trait T: Any {}
impl T for dyn WriteAndSeek {}

#[derive(PartialEq, Eq)]
pub enum SeqMode {
    Linear,
    Random,
}

#[derive(PartialEq, Eq, Debug, Clone, Copy, bincode::Encode, bincode::Decode)]
pub enum CompressionType {
    ZSTD,   // 1 should be default compression ratio
    LZ4,    // 9 should be default compression ratio
    SNAPPY, // Not yet implemented -- IMPLEMENT
    GZIP,   // Please don't use this -- IMPLEMENT
    NAF,    // Not yet supported -- IMPLEMENT
    NONE,   // No Compression -- IMPLEMENT
    XZ,     // Implemented, 6 is default ratio
    BROTLI, // Implemented, 6 is default
    NAFLike // 1 should be default compression rate
}

impl Default for CompressionType {
    fn default() -> CompressionType {
        CompressionType::ZSTD
    }
}

pub const fn default_compression_level(ct: CompressionType) -> i32 {
    match ct {
        CompressionType::ZSTD => 1,
        CompressionType::LZ4 => 9,
        CompressionType::XZ => 6,
        CompressionType::BROTLI => 9,
        CompressionType::NAFLike => 1,
        _ => 3,
    }
}

#[derive(PartialEq, Eq, Debug, Clone)]
pub struct Header {
    pub id: Option<String>,
    pub comment: Option<String>,
    pub citation: Option<String>,
    pub compression_type: CompressionType,
}