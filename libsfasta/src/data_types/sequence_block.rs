// TODO: Make a Sequences struct to handle sequences, like SeqLocs struct...

use crate::data_types::structs::{default_compression_level, CompressionType};

use std::io::{Read, Write};

use xz::read::{XzDecoder, XzEncoder};

#[derive(Debug, Default)]
pub struct SequenceBlock {
    pub seq: Vec<u8>,
}

pub fn zstd_encoder(compression_level: i32) -> zstd::bulk::Compressor<'static> {
    let mut encoder = zstd::bulk::Compressor::new(compression_level).unwrap();
    encoder
        .set_parameter(zstd::stream::raw::CParameter::BlockDelimiters(false))
        .unwrap();
    encoder
        .set_parameter(zstd::stream::raw::CParameter::EnableDedicatedDictSearch(
            true,
        ))
        .unwrap();
    encoder.include_checksum(false).unwrap();
    encoder
        .long_distance_matching(true)
        .expect("Unable to set ZSTD Long Distance Matching");
    encoder
        .window_log(31)
        .expect("Unable to set ZSTD Window Log");
    encoder
        .include_magicbytes(false)
        .expect("Unable to set ZSTD MagicBytes");
    encoder
        .include_contentsize(false)
        .expect("Unable to set ZSTD Content Size Flag");

    encoder
}

impl SequenceBlock {
    pub fn compress(self, compression_type: CompressionType) -> SequenceBlockCompressed {
        let level = default_compression_level(compression_type);
        let mut cseq: Vec<u8> = Vec::with_capacity(4 * 1024 * 1024);

        match compression_type {
            CompressionType::NAFLike => {}
            CompressionType::ZSTD => {
                // TODO: Find a way to reuse this context...
                let mut compressor = zstd_encoder(level);
                compressor.compress_to_buffer(&self.seq, &mut cseq).unwrap();
            }
            CompressionType::LZ4 => {
                let mut compressor = lz4_flex::frame::FrameEncoder::new(cseq);
                compressor
                    .write_all(&self.seq[..])
                    .expect("Unable to compress with LZ4");
                cseq = compressor.finish().unwrap();
            }
            CompressionType::SNAPPY => {
                unimplemented!();
            }
            CompressionType::GZIP => {
                unimplemented!();
            }
            CompressionType::NAF => {
                unimplemented!();
            }
            CompressionType::NONE => {
                unimplemented!();
            }
            CompressionType::XZ => {
                let mut compressor = XzEncoder::new(&self.seq[..], level as u32);
                compressor
                    .read_to_end(&mut cseq)
                    .expect("Unable to XZ compress");
            }
            CompressionType::BROTLI => {
                let mut compressor =
                    brotli::CompressorReader::new(&self.seq[..], 2 * 1024 * 1024, level as u32, 22);
                compressor.read_to_end(&mut cseq).unwrap();
            }
        }

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

#[derive(Debug, Default, bincode::Encode, bincode::Decode)]
pub struct SequenceBlockCompressed {
    pub compressed_seq: Vec<u8>,
}

impl SequenceBlockCompressed {
    pub fn decompress(self, compression_type: CompressionType) -> SequenceBlock {
        // TODO: Capacity here should be set by block-size
        let mut seq: Vec<u8> = Vec::with_capacity(2 * 1024 * 1024);

        match compression_type {
            CompressionType::ZSTD => {
                let mut decoder = zstd::stream::Decoder::new(&self.compressed_seq[..]).unwrap();
                decoder
                    .include_magicbytes(false)
                    .expect("Unable to disable magicbytes in decoder");

                match decoder.read_to_end(&mut seq) {
                    Ok(x) => x,
                    Err(y) => panic!("Unable to decompress block: {:#?}", y),
                };
                // zstd::block::decompress_to_buffer(&self.compressed_seq[..], &mut seq).expect("Unable to decompress");
            }
            CompressionType::XZ => {
                let mut decompressor = XzDecoder::new(&self.compressed_seq[..]);
                decompressor
                    .read_to_end(&mut seq)
                    .expect("Unable to XZ compress");
            }
            _ => {
                unimplemented!()
            }
        };

        SequenceBlock { seq }
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

        let y = x.compress(CompressionType::ZSTD);
        let z = y.decompress(CompressionType::ZSTD);
        assert!(z.seq == test_bytes);
    }
}
