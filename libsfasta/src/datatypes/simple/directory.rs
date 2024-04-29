use crate::*;

use std::num::NonZeroU64;

use pulp::Arch;

#[derive(Debug, Clone, bincode::Encode, bincode::Decode)]
pub struct DirectoryOnDisk
{
    pub index_loc: u64,
    pub ids_loc: u64,
    pub block_index_loc: u64,
    pub seqlocs_loc: u64,
    pub scores_loc: u64,
    pub masking_loc: u64,
    pub headers_loc: u64,
    pub sequences_loc: u64,
}

impl From<Directory> for DirectoryOnDisk
{
    fn from(dir: Directory) -> Self
    {
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
            headers_loc: match dir.headers_loc {
                Some(loc) => loc.get(),
                None => 0,
            },
            sequences_loc: match dir.sequences_loc {
                Some(loc) => loc.get(),
                None => 0,
            },
        }
    }
}

impl DirectoryOnDisk
{
    pub fn sanity_check(&self, buffer_length: u64) -> Result<(), String>
    {
        let values: [u64; 8] = [
            self.index_loc,
            self.ids_loc,
            self.block_index_loc,
            self.seqlocs_loc,
            self.scores_loc,
            self.masking_loc,
            self.headers_loc,
            0,
        ];

        let arch = Arch::new();
        arch.dispatch(|| {
            if values.iter().any(|&x| x > buffer_length) {
                Err(format!(
                    "Some location is outside of buffer: {:?} > {}",
                    values, buffer_length
                ))
            } else {
                Ok(())
            }
        })
    }
}

// , bincode::Encode, bincode::Decode
// Directory should not be encoded, DirectoryOnDisk should be (can use
// .into or .from to get there and back)
#[derive(Debug, Clone, Default)]
pub struct Directory
{
    pub index_loc: Option<NonZeroU64>,
    pub ids_loc: Option<NonZeroU64>,
    pub block_index_loc: Option<NonZeroU64>,
    pub seqlocs_loc: Option<NonZeroU64>,
    pub scores_loc: Option<NonZeroU64>,
    pub masking_loc: Option<NonZeroU64>,
    pub headers_loc: Option<NonZeroU64>,
    pub sequences_loc: Option<NonZeroU64>,
    // TODO: Add pangenome stuff
    // TODO: Add signals stuff
}

impl From<DirectoryOnDisk> for Directory
{
    fn from(dir: DirectoryOnDisk) -> Self
    {
        Directory {
            index_loc: NonZeroU64::new(dir.index_loc),
            ids_loc: NonZeroU64::new(dir.ids_loc),
            block_index_loc: NonZeroU64::new(dir.block_index_loc),
            seqlocs_loc: NonZeroU64::new(dir.seqlocs_loc),
            scores_loc: NonZeroU64::new(dir.scores_loc),
            masking_loc: NonZeroU64::new(dir.masking_loc),
            headers_loc: NonZeroU64::new(dir.headers_loc),
            sequences_loc: NonZeroU64::new(dir.sequences_loc),
        }
    }
}

// impl Default for Directory {
// fn default() -> Self {
// Directory {
// index_loc: None,
// ids_loc: None,
// block_index_loc: None,
// seqlocs_loc: None,
// scores_loc: None,
// masking_loc: None,
// }
// }
// }

impl Directory
{
    // pub fn with_sequences(mut self) -> Self {
    // self.seqlocs_loc = Some(0);
    // self
    // }

    pub fn with_scores(mut self) -> Self
    {
        self.scores_loc = NonZeroU64::new(1);
        self
    }

    pub fn with_index(mut self) -> Self
    {
        self.index_loc = NonZeroU64::new(1);
        self
    }

    pub fn with_masking(mut self) -> Self
    {
        self.masking_loc = NonZeroU64::new(1);
        self
    }

    pub fn dummy(&mut self)
    {
        // Dummy values...
        self.index_loc = NonZeroU64::new(std::u64::MAX);
        self.ids_loc = NonZeroU64::new(std::u64::MAX);
    }
}

impl bincode::Encode for Directory
{
    fn encode<E: bincode::enc::Encoder>(
        &self,
        encoder: &mut E,
    ) -> core::result::Result<(), bincode::error::EncodeError>
    {
        let dir: DirectoryOnDisk = self.clone().into();
        bincode::Encode::encode(&dir, encoder)?;
        Ok(())
    }
}

impl bincode::Decode for Directory
{
    fn decode<D: bincode::de::Decoder>(
        decoder: &mut D,
    ) -> core::result::Result<Self, bincode::error::DecodeError>
    {
        let dir: DirectoryOnDisk = bincode::Decode::decode(decoder)?;
        Ok(dir.into())
    }
}

#[cfg(test)]
mod tests
{
    use super::*;

    #[test]
    pub fn bincode_size_u64()
    {
        let x: u64 = 0;
        let y: u64 = std::u64::MAX;
        let z: u64 = std::u64::MAX - 1;

        let bincode_config =
            bincode::config::standard().with_fixed_int_encoding();

        let encoded_x: Vec<u8> =
            bincode::encode_to_vec(x, bincode_config).unwrap();
        let encoded_y: Vec<u8> =
            bincode::encode_to_vec(y, bincode_config).unwrap();
        let encoded_z: Vec<u8> =
            bincode::encode_to_vec(z, bincode_config).unwrap();

        assert!(encoded_x.len() == encoded_y.len());
        assert!(encoded_x.len() == encoded_z.len());
    }

    #[test]
    pub fn bincode_size_directory_struct()
    {
        let mut directory = Directory {
            index_loc: None,
            ids_loc: None,
            block_index_loc: None,
            seqlocs_loc: None,
            scores_loc: None,
            masking_loc: None,
            headers_loc: None,
            sequences_loc: None,
        };

        let bincode_config =
            bincode::config::standard().with_fixed_int_encoding();

        let dir: DirectoryOnDisk = directory.clone().into();

        let encoded_0: Vec<u8> =
            bincode::encode_to_vec(dir, bincode_config).unwrap();

        directory.index_loc = NonZeroU64::new(std::u64::MAX);
        let dir: DirectoryOnDisk = directory.clone().into();
        let encoded_1: Vec<u8> =
            bincode::encode_to_vec(dir, bincode_config).unwrap();

        directory.scores_loc = NonZeroU64::new(std::u64::MAX);
        let dir: DirectoryOnDisk = directory.into();
        let encoded_2: Vec<u8> =
            bincode::encode_to_vec(dir, bincode_config).unwrap();
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
    pub fn directory_constructors()
    {
        let d = Directory::default().with_scores().with_masking();
        assert!(d.scores_loc == NonZeroU64::new(1));
        assert!(d.masking_loc == NonZeroU64::new(1));

        let d = Directory::default();
        assert!(d.scores_loc.is_none());


    }
}
