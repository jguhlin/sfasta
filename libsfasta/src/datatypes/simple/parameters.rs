use crate::*;

use libcompression::*;

#[derive(Debug, Clone, bincode::Encode, bincode::Decode)]
pub struct Parameters
{
    pub block_size: u32,
    pub compression_type: CompressionType,
    pub compression_dict: Option<Vec<u8>>,
    pub index_compression_type: CompressionType,
}

impl Default for Parameters
{
    fn default() -> Self
    {
        Parameters {
            block_size: 512 * 1024, // 512k
            compression_type: CompressionType::ZSTD,
            compression_dict: None,
            index_compression_type: CompressionType::ZSTD,
        }
    }
}

#[cfg(test)]
mod tests
{
    use super::*;

    #[test]
    pub fn bincode_size_struct()
    {
        let bincode_config =
            bincode::config::standard().with_fixed_int_encoding();

        let mut params = Parameters::default();

        let encoded_0: Vec<u8> =
            bincode::encode_to_vec(&params, bincode_config).unwrap();

        params.compression_type = CompressionType::LZ4;
        let encoded_1: Vec<u8> =
            bincode::encode_to_vec(&params, bincode_config).unwrap();
        assert!(encoded_0.len() == encoded_1.len());
    }
}
