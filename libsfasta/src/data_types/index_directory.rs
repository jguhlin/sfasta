use crate::*;

#[derive(Debug, Clone, bincode::Encode, bincode::Decode, Default)]
pub struct IndexDirectory {
    pub id_index: Option<u64>,
    pub block_index: Option<u64>,
    pub scores_block_index: Option<u64>,
    pub masking_block_index: Option<u64>,
}

impl IndexDirectory {
    pub fn with_ids(mut self) -> Self {
        self.id_index = Some(0);
        self
    }

    pub fn with_blocks(mut self) -> Self {
        self.block_index = Some(0);
        self
    }

    pub fn with_scores_blocks(mut self) -> Self {
        self.scores_block_index = Some(0);
        self
    }

    pub fn with_masking(mut self) -> Self {
        self.masking_block_index = Some(0);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn bincode_size_u64() {
        let x: u64 = 0;
        let y: u64 = std::u64::MAX;
        let z: u64 = std::u64::MAX - 1;

        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        let encoded_x: Vec<u8> = bincode::encode_to_vec(&x, bincode_config).unwrap();
        let encoded_y: Vec<u8> = bincode::encode_to_vec(&y, bincode_config).unwrap();
        let encoded_z: Vec<u8> = bincode::encode_to_vec(&z, bincode_config).unwrap();

        assert!(encoded_x.len() == encoded_y.len());
        assert!(encoded_x.len() == encoded_z.len());
    }

    /*    #[test]
    pub fn bincode_size_directory_struct() {
        let mut directory = IndexDirectory {
            version: 1,
            index_loc: 0,
            seq_loc: Some(0),
            scores_loc: None,
        };

        let encoded_0: Vec<u8> = bincode::serialize(&directory).unwrap();

        directory.index_loc = std::u64::MAX;
        let encoded_1: Vec<u8> = bincode::serialize(&directory).unwrap();

        directory.scores_loc = Some(std::u64::MAX);
        let encoded_2: Vec<u8> = bincode::serialize(&directory).unwrap();

        assert!(encoded_0.len() == encoded_1.len());

        // Option None and Some should be diff sizes!
        assert!(encoded_0.len() != encoded_2.len());
    }

    #[test]
    pub fn directory_constructors() {
        let d = Directory::default().with_scores();
        assert!(d.scores_loc == Some(0));

        let d = Directory::default();
        assert!(d.scores_loc == None);
    } */
}
