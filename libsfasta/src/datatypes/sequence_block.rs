use crate::datatypes::structs::{default_compression_level, CompressionType};
use crate::datatypes::U64BlockStore;
use crate::utils::zstd_decompressor;
use crate::*;

use std::io::{Read, Seek, SeekFrom, Write};

use flate2::write::{GzDecoder, GzEncoder};
use log::error;

#[cfg(not(target_arch = "wasm32"))]
use xz::read::{XzDecoder, XzEncoder};

#[derive(Clone)]
pub struct SequenceBlocks {
    pub block_loc_store: U64BlockStore,
    cache: Option<(u32, Vec<u8>)>,
    cache_sbc: SequenceBlockCompressed,
    pub compression_type: CompressionType,
    pub compression_level: i8,
    pub compression_dict: Option<Vec<u8>>,
    caching: bool,
    block_size: usize,
}

// TODO: Redundant code, clean it up
impl SequenceBlocks {
    pub fn new<R>(
        in_buf: &mut R,
        compression_type: CompressionType,
        compression_dict: Option<Vec<u8>>,
        block_size: usize,
        block_index_location: u64,
    ) -> Self
    where
        R: Read + Seek,
    {
        Self {
            cache: None,
            compression_type,
            compression_level: default_compression_level(compression_type),
            caching: false,
            block_size,
            compression_dict,
            cache_sbc: SequenceBlockCompressed {
                compressed_seq: Vec::with_capacity(block_size),
            },
            block_loc_store: U64BlockStore::from_buffer(in_buf, block_index_location)
                .expect("Unable to open Block Index Store"),
        }
    }

    pub fn without_caching(mut self) -> Self {
        self.caching = false;
        self
    }

    pub fn block_locs<R>(&mut self, mut in_buf: &mut R, block: u32) -> u64
    where
        R: Read + Seek,
    {
        self.block_loc_store.get(in_buf, block as usize)
    }

    pub fn prefetch_block_locs<R>(&mut self, in_buf: &mut R) -> Result<(), String>
    where
        R: Read + Seek,
    {
        self.block_loc_store.prefetch(in_buf);
        Ok(())
    }

    pub fn _get_block<R>(&mut self, mut in_buf: &mut R, block: u32)
    where
        R: Read + Seek,
    {
        if self.cache.is_none() {
            self.cache = Some((block, vec![0; self.block_size]));
        }

        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        self.cache.as_mut().unwrap().0 = block;
        let byte_loc = self.block_locs(&mut in_buf, block);
        in_buf
            .seek(SeekFrom::Start(byte_loc))
            .expect("Unable to work with seek API");

        self.cache_sbc.compressed_seq.clear();
        self.cache_sbc.compressed_seq = bincode::decode_from_std_read(&mut *in_buf, bincode_config)
            .expect("Unable to parse SequenceBlockCompressed");

        let mut zstd_decompressor = if self.compression_type == CompressionType::ZSTD {
            if let Some(compression_dict) = self.compression_dict.as_ref() {
                Some(zstd_decompressor(Some(&compression_dict)))
            } else {
                Some(zstd_decompressor(None))
            }
        } else {
            None
        };

        self.cache.as_mut().unwrap().1.clear();
        self.cache_sbc.decompress_to_buffer(
            self.compression_type,
            &mut self.cache.as_mut().unwrap().1,
            zstd_decompressor.as_mut(),
        );
    }

    pub fn get_block<R>(&mut self, in_buf: &mut R, block: u32) -> &[u8]
    where
        R: Read + Seek,
    {
        if self.cache.is_some() && self.cache.as_ref().unwrap().0 == block {
            self.cache.as_ref().unwrap().1.as_slice()
        } else {
            self._get_block(in_buf, block);
            self.cache.as_ref().unwrap().1.as_slice()
        }
    }

    pub fn get_block_uncached<R>(&mut self, mut in_buf: &mut R, block: u32) -> Vec<u8>
    where
        R: Read + Seek,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        let byte_loc = self.block_locs(&mut in_buf, block);
        in_buf
            .seek(SeekFrom::Start(byte_loc))
            .expect("Unable to work with seek API");

        let sbc: SequenceBlockCompressed = bincode::decode_from_std_read(in_buf, bincode_config)
            .expect("Unable to parse SequenceBlockCompressed");

        let mut zstd_decompressor = if self.compression_type == CompressionType::ZSTD {
            Some(zstd_decompressor(None))
        } else {
            None
        };

        let mut buffer = Vec::with_capacity(self.block_size);
        sbc.decompress_to_buffer(
            self.compression_type,
            &mut buffer,
            zstd_decompressor.as_mut(),
        );

        buffer
    }
}

pub struct SequenceBlock {
    pub seq: Vec<u8>,
}

#[cfg(not(target_arch = "wasm32"))]
pub fn zstd_encoder(
    compression_level: i32,
    dict: Option<Vec<u8>>,
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
    encoder.long_distance_matching(true);
    encoder
}

#[cfg(target_arch = "wasm32")]
pub fn zstd_encoder(compression_level: i32) -> zstd::bulk::Compressor<'static> {
    unimplemented!("ZSTD encoding is not supported on wasm32");
}

impl SequenceBlock {
    pub fn compress(
        self,
        compression_type: CompressionType,
        compression_level: i8,
        zstd_compressor: Option<&mut zstd::bulk::Compressor>,
    ) -> SequenceBlockCompressed {
        #[cfg(test)]
        let mut cseq: Vec<u8> = Vec::with_capacity(512 * 1024);

        #[cfg(not(test))]
        let mut cseq: Vec<u8> = Vec::with_capacity(self.seq.len());

        // Destructure self
        let SequenceBlock { seq } = self;

        match compression_type {
            #[cfg(not(target_arch = "wasm32"))]
            CompressionType::ZSTD => {
                //let mut compressor = zstd_encoder(compression_level as i32);
                let compressor = zstd_compressor.unwrap();
                compressor.compress_to_buffer(&seq, &mut cseq).unwrap();
            }
            #[cfg(target_arch = "wasm32")]
            CompressionType::ZSTD => {
                unimplemented!("ZSTD encoding is not supported on wasm32");
            }
            CompressionType::LZ4 => {
                let mut compressor = lz4_flex::frame::FrameEncoder::new(cseq);
                compressor
                    .write_all(&seq)
                    .expect("Unable to compress with LZ4");
                cseq = compressor.finish().unwrap();
            }
            CompressionType::SNAPPY => {
                let mut compressor = snap::write::FrameEncoder::new(cseq);
                compressor
                    .write_all(&seq)
                    .expect("Unable to compress with Snappy");
                cseq = compressor.into_inner().unwrap();
            }
            CompressionType::GZIP => {
                let mut compressor =
                    GzEncoder::new(cseq, flate2::Compression::new(compression_level as u32));
                compressor
                    .write_all(&seq)
                    .expect("Unable to compress with GZIP");
                cseq = compressor.finish().unwrap();
            }
            CompressionType::NAF => {
                unimplemented!();
            }
            CompressionType::NAFLike => {
                todo!();
            }
            CompressionType::NONE => {
                cseq = seq;
            }
            #[cfg(not(target_arch = "wasm32"))]
            CompressionType::XZ => {
                let mut compressor = XzEncoder::new(&seq[..], compression_level as u32);
                compressor
                    .read_to_end(&mut cseq)
                    .expect("Unable to XZ compress");
            }
            #[cfg(target_arch = "wasm32")]
            CompressionType::XZ => {
                panic!("XZ compression is not supported on wasm32");
            }
            CompressionType::BROTLI => {
                let mut compressor = brotli::CompressorReader::new(
                    &seq[..],
                    2 * 1024 * 1024,
                    compression_level as u32,
                    22,
                );
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

#[derive(bincode::Encode, bincode::Decode, Clone)]
pub struct SequenceBlockCompressed {
    pub compressed_seq: Vec<u8>,
}

impl SequenceBlockCompressed {
    pub fn decompress(
        self,
        compression_type: CompressionType,
        block_size: usize,
        mut zstd_decompressor: Option<&mut zstd::bulk::Decompressor>,
    ) -> SequenceBlock {
        let mut seq: Vec<u8> = Vec::with_capacity(block_size);

        match compression_type {
            CompressionType::NAFLike => {
                todo!();
            }
            #[cfg(not(target_arch = "wasm32"))]
            CompressionType::ZSTD => {
                //let mut decoder = zstd::stream::Decoder::new(&self.compressed_seq[..]).unwrap();
                //decoder
                //    .include_magicbytes(false)
                //    .expect("Unable to disable magicbytes in decoder");

                //match decoder.read_to_end(&mut seq) {
                //    Ok(x) => x,
                //   Err(y) => panic!("Unable to decompress block: {:#?}", y),
                //};
                let zstd = zstd_decompressor.as_mut().unwrap();
                match zstd.decompress_to_buffer(&self.compressed_seq, &mut seq) {
                    Ok(_x) => _x,
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

    pub fn decompress_to_buffer(
        &self,
        compression_type: CompressionType,
        buffer: &mut Vec<u8>,
        mut zstd_decompressor: Option<&mut zstd::bulk::Decompressor>,
    ) {
        buffer.clear();
        match compression_type {
            CompressionType::ZSTD => {
                let zstd = zstd_decompressor.as_mut().unwrap();
                match zstd.decompress_to_buffer(&self.compressed_seq, buffer) {
                    Ok(_x) => _x,
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
            CompressionType::NONE => *buffer = self.compressed_seq.clone(),
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

        let mut compressor = zstd_encoder(3, None);

        let y = x.compress(CompressionType::ZSTD, 3, Some(&mut compressor));
        let mut zstd_decompressor = zstd::bulk::Decompressor::new().unwrap();
        zstd_decompressor.include_magicbytes(false).unwrap();
        let z = y.decompress(
            CompressionType::ZSTD,
            8 * 1024 * 1024,
            Some(zstd_decompressor).as_mut(),
        );
        assert!(z.seq == test_bytes);
    }
}
