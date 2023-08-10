use std::sync::Arc;

#[derive(PartialEq, Eq, Debug, Clone, Copy, bincode::Encode, bincode::Decode)]
#[non_exhaustive]
pub enum CompressionType {
    ZSTD,    // 1 should be default compression ratio
    LZ4,     // 9 should be default compression ratio
    SNAPPY,  // Not yet implemented -- IMPLEMENT
    GZIP,    // Please don't use this -- IMPLEMENT
    NAF,     // Not yet supported -- IMPLEMENT
    NONE,    // No Compression -- IMPLEMENT
    XZ,      // Implemented, 6 is default ratio
    BROTLI,  // Implemented, 6 is default
    NAFLike, // 1 should be default compression rate
    BZIP2,   // Unsupported
    LZMA,    // Unsupported
    RAR,     // Unsupported
}

impl Default for CompressionType {
    fn default() -> CompressionType {
        CompressionType::ZSTD
    }
}

pub const fn default_compression_level(ct: CompressionType) -> i8 {
    match ct {
        CompressionType::ZSTD => 3,
        CompressionType::LZ4 => 9,
        CompressionType::XZ => 6,
        CompressionType::BROTLI => 9,
        CompressionType::NAFLike => 1,
        _ => 3,
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn zstd_encoder(
    compression_level: i32,
    dict: &Option<Arc<Vec<u8>>>,
) -> zstd::bulk::Compressor<'static> {
    let mut encoder = if let Some(dict) = dict {
        zstd::bulk::Compressor::with_dictionary(compression_level, &dict).unwrap()
    } else {
        zstd::bulk::Compressor::new(compression_level).unwrap()
    };
    encoder.include_checksum(false).unwrap();
    encoder
        .include_magicbytes(false)
        .expect("Unable to set ZSTD MagicBytes");
    encoder
        .include_contentsize(false)
        .expect("Unable to set ZSTD Content Size Flag");
    encoder
        .long_distance_matching(true)
        .expect("Unable to set long_distance_matching");
    encoder
}

#[cfg(not(target_arch = "wasm32"))]
pub fn zstd_decompressor<'a>(dict: Option<&[u8]>) -> zstd::bulk::Decompressor<'a> {
    let mut zstd_decompressor = if let Some(dict) = dict {
        zstd::bulk::Decompressor::with_dictionary(dict)
    } else {
        zstd::bulk::Decompressor::new()
    }
    .unwrap();

    zstd_decompressor
        .include_magicbytes(false)
        .expect("Failed to set magicbytes");
    zstd_decompressor
}

#[cfg(target_arch = "wasm32")]
pub fn zstd_decompressor<'a>(dict: Option<&[u8]>) -> zstd::bulk::Decompressor<'a> {
    unimplemented!("ZSTD decoding is not supported on wasm32");
}

#[cfg(target_arch = "wasm32")]
pub fn zstd_encoder(compression_level: i32) -> zstd::bulk::Compressor<'static> {
    unimplemented!("ZSTD encoding is not supported on wasm32");
}
