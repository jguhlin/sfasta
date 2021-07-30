use crate::*;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Directory {
    pub index_loc: Option<u64>,
    pub ids_loc: u64,
    pub block_index_loc: u64,
    pub seqlocs_loc: u64,
    pub scores_loc: Option<u64>,
    pub masking_loc: Option<u64>,
    pub id_blocks_index_loc: Option<u64>,
    pub seqloc_blocks_index_loc: Option<u64>,
    pub index_plan_loc: Option<u64>,
    pub index_bitpacked_loc: Option<u64>,
}

impl Default for Directory {
    fn default() -> Self {
        Directory {
            index_loc: Some(0),
            ids_loc: 0,
            block_index_loc: 0,
            seqlocs_loc: 0,
            scores_loc: None,
            masking_loc: None,
            id_blocks_index_loc: Some(0),
            seqloc_blocks_index_loc: Some(0),
        }
    }
}

impl Directory {
    /* pub fn with_sequences(mut self) -> Self {
        self.seqlocs_loc = Some(0);
        self
    } */

    pub fn with_scores(mut self) -> Self {
        self.scores_loc = Some(0);
        self
    }

    pub fn with_index(mut self) -> Self {
        self.index_loc = Some(0);
        self.id_blocks_index_loc = Some(0);
        self
    }

    pub fn with_masking(mut self) -> Self {
        self.masking_loc = Some(0);
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

        let encoded_x: Vec<u8> = bincode::serialize(&x).unwrap();
        let encoded_y: Vec<u8> = bincode::serialize(&y).unwrap();
        let encoded_z: Vec<u8> = bincode::serialize(&z).unwrap();

        assert!(encoded_x.len() == encoded_y.len());
        assert!(encoded_x.len() == encoded_z.len());
    }

    #[test]
    pub fn bincode_size_directory_struct() {
        let mut directory = Directory {
            index_loc: Some(0),
            ids_loc: 0,
            block_index_loc: 0,
            seqlocs_loc: 0,
            scores_loc: None,
            masking_loc: None,
            id_blocks_index_loc: Some(0),
            seqloc_blocks_index_loc: Some(0),
        };

        let encoded_0: Vec<u8> = bincode::serialize(&directory).unwrap();

        directory.index_loc = Some(std::u64::MAX);
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
    }
}
