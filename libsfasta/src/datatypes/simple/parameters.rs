use crate::*;

use libcompression::*;

#[derive(Debug, Clone, bincode::Encode, bincode::Decode)]
pub struct Parameters {
    pub block_size: u32,
}

impl Default for Parameters {
    fn default() -> Self {
        Parameters {
            block_size: 512 * 1024, // 512k
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn bincode_size_struct() {
        let bincode_config =
            bincode::config::standard().with_fixed_int_encoding();

        let mut params = Parameters::default();

        let encoded_0: Vec<u8> =
            bincode::encode_to_vec(&params, bincode_config).unwrap();

        let encoded_1: Vec<u8> =
            bincode::encode_to_vec(&params, bincode_config).unwrap();
        assert!(encoded_0.len() == encoded_1.len());
    }
}
