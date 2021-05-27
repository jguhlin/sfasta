use crate::structs::{default_compression_level, CompressionType};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use xz::read::{XzDecoder, XzEncoder};

#[derive(Clone, Debug, Default)]
pub struct SequenceBlock {
    //    pub compression_type: CompressionType,
    pub seq: Vec<u8>,
}

impl SequenceBlock {
    pub fn compress(self, compression_type: CompressionType) -> SequenceBlockCompressed {
        let level = default_compression_level(compression_type);
        let mut cseq: Vec<u8> = Vec::with_capacity(4 * 1024 * 1024);

        match compression_type {
            CompressionType::ZSTD => {
                let mut encoder = zstd::stream::Encoder::new(cseq, level).unwrap();
                encoder
                    .long_distance_matching(true)
                    .expect("Unable to set ZSTD Long Distance Matching");
                encoder
                    .include_magicbytes(false)
                    .expect("Unable to set ZSTD MagicBytes");
                encoder
                    .include_contentsize(false)
                    .expect("Unable to set ZSTD Content Size Flag");
                encoder
                    .write_all(&self.seq[..])
                    .expect("Unable to write sequence to ZSTD compressor");
                cseq = encoder.finish().unwrap();
            }
            CompressionType::LZ4 => { 
                let mut compressor = lz4_flex::frame::FrameEncoder::new(cseq);
                compressor.write_all(&self.seq[..]).expect("Unable to compress with LZ4");
                cseq = compressor.finish().unwrap();
            }
            CompressionType::SNAPPY => { unimplemented!(); }
            CompressionType::GZIP => { unimplemented!(); }
            CompressionType::NAF => { unimplemented!(); }
            CompressionType::NONE => { unimplemented!(); }
            CompressionType::XZ => {
                let mut compressor = XzEncoder::new(&self.seq[..], level as u32);
                compressor
                    .read_to_end(&mut cseq)
                    .expect("Unable to XZ compress");
            },
            CompressionType::BROTLI => { unimplemented!(); },
        }

        SequenceBlockCompressed {
            compressed_seq: cseq,
        }
    }

    pub fn len(&self) -> usize {
        self.seq.len()
    }

    // Convenience Function
    /*    pub fn with_compression_type(mut self, compression_type: CompressionType) -> Self {
        self.compression_type = compression_type;
        self
    } */
}

#[derive(Clone, Deserialize, Serialize, Debug, Default)]
pub struct SequenceBlockCompressed {
    #[serde(with = "serde_bytes")]
    pub compressed_seq: Vec<u8>,
}

impl SequenceBlockCompressed {
    pub fn decompress(&self) -> SequenceBlock {
        // let mut decompressor = zstd::block::Decompressor::new();
        let mut decoder = zstd::stream::Decoder::new(&self.compressed_seq[..]).unwrap();
        // encoder.multithread(1);
        decoder
            .include_magicbytes(false)
            .expect("Unable to disable magicbytes in decoder");

        // TODO: Capacity here should be set by block-size
        //let seq: Vec<u8> = decompressor.decompress(&self.compressed_seq[..], 128 * 1024 * 1024).expect("Unable to decompress block");
        let mut seq: Vec<u8> = Vec::with_capacity(4 * 1024 * 1024);
        match decoder.read_to_end(&mut seq) {
            Ok(x) => x,
            Err(y) => panic!("Unable to decompress block: {:#?}", y),
        };

        SequenceBlock {
            seq,
            // compression_type: self.compression_type,
        }
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
        let test_bytes = b"abatcacgactac".to_vec();
        let x = SequenceBlockCompressed {
            compressed_seq: test_bytes.clone(),
            ..Default::default()
        };

        let encoded = bincode::serialize(&x).unwrap();
        let decoded: SequenceBlockCompressed = bincode::deserialize(&encoded).unwrap();
        assert!(decoded.compressed_seq == x.compressed_seq);
        assert!(decoded.compressed_seq == test_bytes);
    }

    #[test]
    pub fn test_compress_and_decompress() {
        let test_bytes = b"abatcacgactac".to_vec();
        let x = SequenceBlock {
            seq: test_bytes.clone(),
            ..Default::default()
        };

        let y = x.compress(CompressionType::ZSTD);
        let z = y.decompress();
        assert!(z.seq == test_bytes);
    }
}
