use crate::*;

use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
struct DirectoryOnDisk {
    pub index_loc: u64,
    pub ids_loc: u64,
    pub block_index_loc: u64,
    pub seqlocs_loc: u64,
    pub scores_loc: u64,
    pub masking_loc: u64,
    pub id_blocks_index_loc: u64,
    pub seqloc_blocks_index_loc: u64,
    pub index_plan_loc: u64,
    pub index_bitpacked_loc: u64,
}

impl From<Directory> for DirectoryOnDisk {
    fn from(dir: Directory) -> Self {
        DirectoryOnDisk {
            index_loc: match dir.index_loc {
                Some(loc) => loc.get(),
                None => 0,
            },
            ids_loc: match dir.ids_loc {
                Some(loc) => loc.get(),
                None => 0,
            },
            block_index_loc: match dir.block_index_loc {
                Some(loc) => loc.get(),
                None => 0,
            },
            seqlocs_loc: match dir.seqlocs_loc {
                Some(loc) => loc.get(),
                None => 0,
            },
            scores_loc: match dir.scores_loc {
                Some(loc) => loc.get(),
                None => 0,
            },
            masking_loc: match dir.masking_loc {
                Some(loc) => loc.get(),
                None => 0,
            },
            id_blocks_index_loc: match dir.id_blocks_index_loc {
                Some(loc) => loc.get(),
                None => 0,
            },
            seqloc_blocks_index_loc: match dir.seqloc_blocks_index_loc {
                Some(loc) => loc.get(),
                None => 0,
            },
            index_plan_loc: match dir.index_plan_loc {
                Some(loc) => loc.get(),
                None => 0,
            },
            index_bitpacked_loc: match dir.index_bitpacked_loc {
                Some(loc) => loc.get(),
                None => 0,
            },
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(into = "DirectoryOnDisk")]
#[serde(from = "DirectoryOnDisk")]
pub struct Directory {
    pub index_loc: Option<NonZeroU64>,
    pub ids_loc: Option<NonZeroU64>,
    pub block_index_loc: Option<NonZeroU64>,
    pub seqlocs_loc: Option<NonZeroU64>,
    pub scores_loc: Option<NonZeroU64>,
    pub masking_loc: Option<NonZeroU64>,
    pub id_blocks_index_loc: Option<NonZeroU64>,
    pub seqloc_blocks_index_loc: Option<NonZeroU64>,
    pub index_plan_loc: Option<NonZeroU64>,
    pub index_bitpacked_loc: Option<NonZeroU64>,
    // TODO: Add pangenome stuff
}

impl From<DirectoryOnDisk> for Directory {
    fn from(dir: DirectoryOnDisk) -> Self {
        Directory {
            index_loc: NonZeroU64::new(dir.index_loc),
            ids_loc: NonZeroU64::new(dir.ids_loc),
            block_index_loc: NonZeroU64::new(dir.block_index_loc),
            seqlocs_loc: NonZeroU64::new(dir.seqlocs_loc),
            scores_loc: NonZeroU64::new(dir.scores_loc),
            masking_loc: NonZeroU64::new(dir.masking_loc),
            id_blocks_index_loc: NonZeroU64::new(dir.id_blocks_index_loc),
            seqloc_blocks_index_loc: NonZeroU64::new(dir.seqloc_blocks_index_loc),
            index_plan_loc: NonZeroU64::new(dir.index_plan_loc),
            index_bitpacked_loc: NonZeroU64::new(dir.index_bitpacked_loc),
        }
    }
}

impl Default for Directory {
    fn default() -> Self {
        Directory {
            index_loc: None,
            ids_loc: None,
            block_index_loc: None,
            seqlocs_loc: None,
            scores_loc: None,
            masking_loc: None,
            id_blocks_index_loc: None,
            seqloc_blocks_index_loc: None,
            index_bitpacked_loc: None,
            index_plan_loc: None,
        }
    }
}

impl Directory {
    /* pub fn with_sequences(mut self) -> Self {
        self.seqlocs_loc = Some(0);
        self
    } */

    pub fn with_scores(mut self) -> Self {
        self.scores_loc = NonZeroU64::new(1);
        self
    }

    pub fn with_index(mut self) -> Self {
        self.index_loc = NonZeroU64::new(1);
        self.id_blocks_index_loc = NonZeroU64::new(1);
        self
    }

    pub fn with_masking(mut self) -> Self {
        self.masking_loc = NonZeroU64::new(1);
        self
    }

    pub fn dummy(&mut self) {
        // Dummy values...
        self.seqloc_blocks_index_loc = NonZeroU64::new(std::u64::MAX);
        self.id_blocks_index_loc = NonZeroU64::new(std::u64::MAX);
        self.index_loc = NonZeroU64::new(std::u64::MAX);
        self.ids_loc = NonZeroU64::new(std::u64::MAX);
        self.index_plan_loc = NonZeroU64::new(std::u64::MAX);
        self.index_bitpacked_loc = NonZeroU64::new(std::u64::MAX);
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
            index_loc: None,
            ids_loc: None,
            block_index_loc: None,
            seqlocs_loc: None,
            scores_loc: None,
            masking_loc: None,
            id_blocks_index_loc: None,
            seqloc_blocks_index_loc: None,
            index_bitpacked_loc: None,
            index_plan_loc: None,
        };

        let encoded_0: Vec<u8> = bincode::serialize(&directory).unwrap();

        directory.index_loc = NonZeroU64::new(std::u64::MAX);
        let encoded_1: Vec<u8> = bincode::serialize(&directory).unwrap();

        directory.scores_loc = NonZeroU64::new(std::u64::MAX);
        let encoded_2: Vec<u8> = bincode::serialize(&directory).unwrap();
        println!(
            "{} {} {}",
            encoded_0.len(),
            encoded_1.len(),
            encoded_2.len()
        );

        assert!(encoded_0.len() == encoded_1.len());
        assert!(encoded_0.len() == encoded_2.len());
    }

    #[test]
    pub fn directory_constructors() {
        let d = Directory::default().with_scores();
        assert!(d.scores_loc == NonZeroU64::new(1));

        let d = Directory::default();
        assert!(d.scores_loc == None);
    }
}
