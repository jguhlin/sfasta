use crate::*;

use super::structs::CompressionType;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, bincode::Encode, bincode::Decode)]
pub struct Parameters {
    pub block_size: u32,
    pub compression_type: CompressionType,
    pub index_compression_type: CompressionType,
    pub index_chunk_size: u32,
    pub seqlocs_chunk_size: u32,
}

impl Default for Parameters {
    fn default() -> Self {
        Parameters {
            block_size: 4 * 1024 * 1024, // 4 Mb
            compression_type: CompressionType::ZSTD,
            index_compression_type: CompressionType::LZ4,
            index_chunk_size: 64 * 1024,   // 64k
            seqlocs_chunk_size: 64 * 1024, // 64k
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn bincode_size_struct() {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        let mut params = Parameters::default();

        let encoded_0: Vec<u8> = bincode::serde::encode_to_vec(&params, bincode_config).unwrap();

        params.compression_type = CompressionType::LZ4;
        let encoded_1: Vec<u8> = bincode::serde::encode_to_vec(&params, bincode_config).unwrap();
        assert!(encoded_0.len() == encoded_1.len());
    }
}
