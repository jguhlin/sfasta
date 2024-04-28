use serde::{Deserialize, Serialize};

use libcompression::CompressionConfig;

impl CompressionProfile
{
    pub fn default() -> Self
    {
        serde_yml::from_str(include_str!(
            "../../../compression_profiles/default.yaml"
        ))
        .unwrap()
    }

    pub fn fast() -> Self
    {
        serde_yml::from_str(include_str!(
            "../../../compression_profiles/fast.yaml"
        ))
        .unwrap()
    }

    pub fn fastest() -> Self
    {
        serde_yml::from_str(include_str!(
            "../../../compression_profiles/fastest.yaml"
        ))
        .unwrap()
    }

    pub fn small() -> Self
    {
        serde_yml::from_str(include_str!(
            "../../../compression_profiles/small.yaml"
        ))
        .unwrap()
    }

    pub fn smallest() -> Self
    {
        serde_yml::from_str(include_str!(
            "../../../compression_profiles/smallest.yaml"
        ))
        .unwrap()
    }
}

impl Default for CompressionProfile
{
    fn default() -> Self
    {
        Self::default()
    }
}

#[derive(Serialize, Deserialize)]
pub struct CompressionProfile
{
    data: DataCompressionProfile,
    index: IndexCompressionProfile,
    seqlocs: CompressionConfig,
    id_index: CompressionConfig,
}

#[derive(Default, Serialize, Deserialize)]
pub struct DataCompressionProfile
{
    pub ids: CompressionConfig,
    pub headers: CompressionConfig,
    pub sequence: CompressionConfig,
    pub masking: CompressionConfig,
    pub quality: CompressionConfig,
    pub signals: CompressionConfig,
    pub modifications: CompressionConfig,
}

#[derive(Default, Serialize, Deserialize)]
pub struct IndexCompressionProfile
{
    pub ids: CompressionConfig,
    pub headers: CompressionConfig,
    pub sequence: CompressionConfig,
    pub masking: CompressionConfig,
    pub quality: CompressionConfig,
    pub signals: CompressionConfig,
    pub modifications: CompressionConfig,
}

#[cfg(test)]

mod tests
{
    use super::*;
    use libcompression::CompressionType;

    #[test]
    fn test_parsing_compression_profile()
    {
        // Load up compression_profiles/default.yaml
        let default_profile = serde_yml::from_str::<CompressionProfile>(
            include_str!("../../../compression_profiles/default.yaml"),
        )
        .unwrap();
    }
}
