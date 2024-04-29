use serde::{Deserialize, Serialize};

use libcompression::{CompressionConfig, CompressionType};

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

    pub fn set_global(ct: CompressionType, level: i8) -> Self
    {
        let config = CompressionConfig::new()
            .with_compression_type(ct)
            .with_compression_level(level);
        Self {
            block_size: 512, // Not really read after this point...
            data: DataCompressionProfile::splat(config.clone()),
            index: IndexCompressionProfile::splat(config.clone()),
            seqlocs: config.clone(),
            id_index: config,
        }
    }
}

impl Default for CompressionProfile
{
    fn default() -> Self
    {
        Self::default()
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
pub struct CompressionProfile
{
    pub block_size: u32,
    pub data: DataCompressionProfile,
    pub index: IndexCompressionProfile,
    pub seqlocs: CompressionConfig,
    pub id_index: CompressionConfig,
}

#[derive(Default, Serialize, Deserialize, PartialEq, Eq, Debug)]
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

impl DataCompressionProfile
{
    fn splat(cc: CompressionConfig) -> Self
    {
        Self {
            ids: cc.clone(),
            headers: cc.clone(),
            sequence: cc.clone(),
            masking: cc.clone(),
            quality: cc.clone(),
            signals: cc.clone(),
            modifications: cc.clone(),
        }
    }
}

#[derive(Default, Serialize, Deserialize, PartialEq, Eq, Debug)]
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

impl IndexCompressionProfile
{
    fn splat(cc: CompressionConfig) -> Self
    {
        Self {
            ids: cc.clone(),
            headers: cc.clone(),
            sequence: cc.clone(),
            masking: cc.clone(),
            quality: cc.clone(),
            signals: cc.clone(),
            modifications: cc.clone(),
        }
    }
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

    #[test]
    fn test_splat()
    {
        let cc = CompressionConfig::new()
            .with_compression_type(CompressionType::BZIP2)
            .with_compression_level(9);
        let dcp = DataCompressionProfile::splat(cc.clone());
        assert_eq!(dcp.ids, cc);
        assert_eq!(dcp.headers, cc);
        assert_eq!(dcp.sequence, cc);
        assert_eq!(dcp.masking, cc);
        assert_eq!(dcp.quality, cc);
        assert_eq!(dcp.signals, cc);
        assert_eq!(dcp.modifications, cc);
    }

    #[test]
    fn test_profiles() 
    {
        // Should not equal default
        assert_ne!(CompressionProfile::default(), CompressionProfile::fast());
        assert_ne!(CompressionProfile::default(), CompressionProfile::fastest());
        assert_ne!(CompressionProfile::default(), CompressionProfile::small());
        assert_ne!(CompressionProfile::default(), CompressionProfile::smallest());
    }
}
