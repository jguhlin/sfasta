use crate::*;

use super::structs::CompressionType;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Parameters {
    pub block_size: u32,
    pub compression_type: CompressionType,
    pub index_compression_type: CompressionType,
}

impl Default for Parameters {
    fn default() -> Self {
        Parameters {
            block_size: 4 * 1024 * 1024, // 4 Mb
            compression_type: CompressionType::ZSTD,
            index_compression_type: CompressionType::LZ4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn bincode_size_struct() {
        let mut params = Parameters::default();

        let encoded_0: Vec<u8> = bincode::serialize(&params).unwrap();

        params.compression_type = CompressionType::LZ4;
        let encoded_1: Vec<u8> = bincode::serialize(&params).unwrap();
        assert!(encoded_0.len() == encoded_1.len());
    }
}
