use crate::datatypes::structs::{default_compression_level, CompressionType};

use std::io::{Read, Seek, SeekFrom, Write};

use flate2::write::{GzDecoder, GzEncoder};
use log::error;

#[cfg(not(target_arch = "wasm32"))]
use xz::read::{XzDecoder, XzEncoder};

pub struct SequenceBlocks {
    pub block_locs: Vec<u64>,
    cache: Option<(u32, Vec<u8>)>,
    pub compression_type: CompressionType,
    pub compression_level: i8,

}

impl SequenceBlocks {
    pub fn new(block_locs: Vec<u64>, compression_type: CompressionType) -> Self {
        SequenceBlocks {
            block_locs,
            cache: None,
            compression_type,
            compression_level: default_compression_level(compression_type),
        }
    }

    pub fn get_block<R>(&mut self, in_buf: &mut R, block: u32) -> &[u8]
    where
        R: Read + Seek,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        if self.cache.is_some() && self.cache.as_ref().unwrap().0 == block {
            return self.cache.as_ref().unwrap().1.as_slice();
        } else {
            if self.cache == None {
                self.cache = Some((block, Vec::with_capacity(2 * 1024 * 1024)));
            }
            let byte_loc = self.block_locs[block as usize];
            in_buf
                .seek(SeekFrom::Start(byte_loc))
                .expect("Unable to work with seek API");
            let sbc: SequenceBlockCompressed =
                bincode::decode_from_std_read(&mut *in_buf, bincode_config)
                    .expect("Unable to parse SequenceBlockCompressed");

            let cache = self.cache.as_mut().unwrap();
            sbc.decompress_to_buffer(self.compression_type, &mut cache.1);
            cache.0 = block;
            return self.cache.as_ref().unwrap().1.as_slice();
        }
    }
}

#[derive(Debug, Default)]
pub struct SequenceBlock {
    pub seq: Vec<u8>,
}

#[cfg(not(target_arch = "wasm32"))]
pub fn zstd_encoder(compression_level: i32) -> zstd::bulk::Compressor<'static> {
    let mut encoder = zstd::bulk::Compressor::new(compression_level).unwrap();
    encoder
        .set_parameter(zstd::stream::raw::CParameter::BlockDelimiters(false))
        .unwrap();
    encoder
        .set_parameter(zstd::stream::raw::CParameter::EnableDedicatedDictSearch(
            false,
        ))
        .unwrap();
    encoder.include_checksum(false).unwrap();
    encoder
        .long_distance_matching(true)
        .expect("Unable to set ZSTD Long Distance Matching");
    encoder
        .window_log(27)
        .expect("Unable to set ZSTD Window Log");
    encoder
        .include_magicbytes(false)
        .expect("Unable to set ZSTD MagicBytes");
    encoder
        .include_contentsize(false)
        .expect("Unable to set ZSTD Content Size Flag");
    encoder.include_dictid(false).expect("Unable to set dictid");
    encoder
}

#[cfg(target_arch = "wasm32")]
pub fn zstd_encoder(compression_level: i32) -> zstd::bulk::Compressor<'static> {
    unimplemented!("ZSTD encoding is not supported on wasm32");
}

impl SequenceBlock {
    pub fn compress(self, compression_type: CompressionType, compression_level: i8) -> SequenceBlockCompressed {
        let mut cseq: Vec<u8> = Vec::with_capacity(self.seq.len());

        //debug!("Compressing sequence block with length: {}", self.seq.len());

        match compression_type {
            CompressionType::NAFLike => {
                todo!();
            }
            #[cfg(not(target_arch = "wasm32"))]
            CompressionType::ZSTD => {
                // TODO: Find a way to reuse this context...
                let mut compressor = zstd_encoder(compression_level as i32);
                compressor.compress_to_buffer(&self.seq, &mut cseq).unwrap();
            }
            #[cfg(target_arch = "wasm32")]
            CompressionType::ZSTD => {
                unimplemented!("ZSTD encoding is not supported on wasm32");
            }
            CompressionType::LZ4 => {
                let mut compressor = lz4_flex::frame::FrameEncoder::new(cseq);
                compressor
                    .write_all(&self.seq[..])
                    .expect("Unable to compress with LZ4");
                cseq = compressor.finish().unwrap();
            }
            CompressionType::SNAPPY => {
                let mut compressor = snap::write::FrameEncoder::new(cseq);
                compressor
                    .write_all(&self.seq[..])
                    .expect("Unable to compress with Snappy");
                cseq = compressor.into_inner().unwrap();
            }
            CompressionType::GZIP => {
                let mut compressor = GzEncoder::new(cseq, flate2::Compression::new(compression_level as u32));
                compressor
                    .write_all(&self.seq[..])
                    .expect("Unable to compress with GZIP");
                cseq = compressor.finish().unwrap();
            }
            CompressionType::NAF => {
                unimplemented!();
            }
            CompressionType::NONE => {
                cseq = self.seq;
            }
            #[cfg(not(target_arch = "wasm32"))]
            CompressionType::XZ => {
                let mut compressor = XzEncoder::new(&self.seq[..], compression_level as u32);
                compressor
                    .read_to_end(&mut cseq)
                    .expect("Unable to XZ compress");
            }
            #[cfg(target_arch = "wasm32")]
            CompressionType::XZ => {
                panic!("XZ compression is not supported on wasm32");
            }
            CompressionType::BROTLI => {
                let mut compressor =
                    brotli::CompressorReader::new(&self.seq[..], 2 * 1024 * 1024, compression_level as u32, 22);
                compressor.read_to_end(&mut cseq).unwrap();
            }
            _ => {
                error!("Unsupported compression type: {:?}", compression_type);
                panic!("Unsupported compression type: {:?}", compression_type);
            }
        }

        //debug!("Compressed sequence block to length: {}", cseq.len());

        SequenceBlockCompressed {
            compressed_seq: cseq,
        }
    }

    pub fn len(&self) -> usize {
        self.seq.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seq.is_empty()
    }
}

#[derive(Debug, Default, bincode::Encode, bincode::Decode, Clone)]
pub struct SequenceBlockCompressed {
    pub compressed_seq: Vec<u8>,
}

impl SequenceBlockCompressed {
    pub fn decompress(self, compression_type: CompressionType) -> SequenceBlock {
        // TODO: Capacity here should be set by block-size
        let mut seq: Vec<u8> = Vec::with_capacity(4 * 1024 * 1024);

        match compression_type {
            CompressionType::NAFLike => {
                todo!();
            }
            #[cfg(not(target_arch = "wasm32"))]
            CompressionType::ZSTD => {
                let mut decoder = zstd::stream::Decoder::new(&self.compressed_seq[..]).unwrap();
                decoder
                    .include_magicbytes(false)
                    .expect("Unable to disable magicbytes in decoder");

                match decoder.read_to_end(&mut seq) {
                    Ok(x) => x,
                    Err(y) => panic!("Unable to decompress block: {:#?}", y),
                };
            }

            #[cfg(target_arch = "wasm32")]
            CompressionType::ZSTD => {
                let mut decoder = ruzstd::StreamingDecoder::new(&self.compressed_seq[..]).unwrap();

                match decoder.read_to_end(&mut seq) {
                    Ok(x) => x,
                    Err(y) => panic!("Unable to decompress block: {:#?}", y),
                };
            }

            #[cfg(not(target_arch = "wasm32"))]
            CompressionType::XZ => {
                let mut decompressor = XzDecoder::new(&self.compressed_seq[..]);
                decompressor
                    .read_to_end(&mut seq)
                    .expect("Unable to XZ compress");
            }
            #[cfg(target_arch = "wasm32")]
            CompressionType::XZ => {
                panic!("XZ compression is not supported on wasm32");
            }

            CompressionType::BROTLI => {
                let mut decompressor =
                    brotli::Decompressor::new(&self.compressed_seq[..], 2 * 1024 * 1024);
                decompressor.read_to_end(&mut seq).unwrap();
            }
            CompressionType::LZ4 => {
                let mut decompressor = lz4_flex::frame::FrameDecoder::new(&self.compressed_seq[..]);
                decompressor
                    .read_to_end(&mut seq)
                    .expect("Unable to decompress with LZ4");
            }
            CompressionType::SNAPPY => {
                let mut decompressor = snap::read::FrameDecoder::new(&self.compressed_seq[..]);
                decompressor
                    .read_to_end(&mut seq)
                    .expect("Unable to decompress with Snappy");
            }
            CompressionType::GZIP => {
                let mut decompressor = GzDecoder::new(&mut seq);
                decompressor
                    .write_all(&self.compressed_seq[..])
                    .expect("Unable to decompress with GZIP");
            }
            CompressionType::NONE => seq = self.compressed_seq,
            _ => {
                unimplemented!()
            }
        };

        SequenceBlock { seq }
    }

    pub fn decompress_to_buffer(self, compression_type: CompressionType, buffer: &mut Vec<u8>) {
        buffer.clear();
        match compression_type {
            CompressionType::ZSTD => {
                let mut decoder = zstd::stream::Decoder::new(&self.compressed_seq[..]).unwrap();
                decoder
                    .include_magicbytes(false)
                    .expect("Unable to disable magicbytes in decoder");

                match decoder.read_to_end(buffer) {
                    Ok(x) => x,
                    Err(y) => panic!("Unable to decompress block: {:#?}", y),
                };
            }
            CompressionType::XZ => {
                let mut decompressor = XzDecoder::new(&self.compressed_seq[..]);
                decompressor
                    .read_to_end(buffer)
                    .expect("Unable to XZ compress");
            }
            CompressionType::BROTLI => {
                let mut decompressor =
                    brotli::Decompressor::new(&self.compressed_seq[..], 2 * 1024 * 1024);
                decompressor.read_to_end(buffer).unwrap();
            }
            CompressionType::LZ4 => {
                let mut decompressor = lz4_flex::frame::FrameDecoder::new(&self.compressed_seq[..]);
                decompressor
                    .read_to_end(buffer)
                    .expect("Unable to decompress with LZ4");
            }
            CompressionType::SNAPPY => {
                let mut decompressor = snap::read::FrameDecoder::new(&self.compressed_seq[..]);
                decompressor
                    .read_to_end(buffer)
                    .expect("Unable to decompress with Snappy");
            }
            CompressionType::GZIP => {
                let mut decompressor = GzDecoder::new(buffer);
                decompressor
                    .write_all(&self.compressed_seq[..])
                    .expect("Unable to decompress with GZIP");
            }
            CompressionType::NONE => *buffer = self.compressed_seq,
            _ => {
                unimplemented!()
            }
        };
    }

    // Convenience Function
    /*    pub fn with_compression_type(mut self, compression_type: CompressionType) -> Self {
        self.compression_type = compression_type;
        self
    } */
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn test_encode_and_decode() {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        let test_bytes = b"abatcacgactac".to_vec();
        let x = SequenceBlockCompressed {
            compressed_seq: test_bytes.clone(),
        };

        let encoded = bincode::encode_to_vec(&x, bincode_config).unwrap();
        let decoded: SequenceBlockCompressed = bincode::decode_from_slice(&encoded, bincode_config)
            .unwrap()
            .0;
        assert!(decoded.compressed_seq == x.compressed_seq);
        assert!(decoded.compressed_seq == test_bytes);
    }

    #[test]
    pub fn test_compress_and_decompress() {
        let test_bytes = b"abatcacgactac".to_vec();
        let x = SequenceBlock {
            seq: test_bytes.clone(),
        };

        let y = x.compress(CompressionType::ZSTD, 3);
        let z = y.decompress(CompressionType::ZSTD);
        assert!(z.seq == test_bytes);
    }
}
