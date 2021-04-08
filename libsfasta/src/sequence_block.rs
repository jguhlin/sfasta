use crate::parameters::Parameters;
use crate::structs::{default_compression_level, CompressionType};
use serde::{Deserialize, Serialize};
use std::rc::Rc;
use zstd;

#[derive(Clone, Debug, Default)]
pub struct SequenceBlock {
    pub compression_type: CompressionType,
    pub seq: Vec<u8>,
}

impl SequenceBlock {
    pub fn compress(&self) -> SequenceBlockCompressed {
        let level = default_compression_level(self.compression_type);
        let cseq = match zstd::stream::encode_all(&self.seq[..], level) {
            Ok(x) => x,
            Err(x) => panic!("{:#?}", x),
        };

        SequenceBlockCompressed {
            compression_type: self.compression_type,
            compressed_seq: cseq,
        }
    }

    pub fn len(&self) -> usize {
        self.seq.len()
    }

    // Convenience Function
    pub fn with_compression_type(mut self, compression_type: CompressionType) -> Self {
        self.compression_type = compression_type;
        self
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct SequenceBlockCompressed {
    pub compressed_seq: Vec<u8>,

    #[serde(skip)] // This is serialized in the parameters field already... Here for convenience...
    pub compression_type: CompressionType,
}

impl SequenceBlockCompressed {
    pub fn decompress(&self) -> SequenceBlock {
        let decompressed = match zstd::stream::decode_all(&self.compressed_seq[..]) {
            Ok(x) => x,
            Err(y) => panic!("{:#?}", y),
        };

        SequenceBlock {
            seq: decompressed,
            compression_type: self.compression_type,
        }
    }

    // Convenience Function
    pub fn with_compression_type(mut self, compression_type: CompressionType) -> Self {
        self.compression_type = compression_type;
        self
    }
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

        let y = x.compress();
        let z = y.decompress();
        assert!(z.seq == test_bytes);
    }
}
