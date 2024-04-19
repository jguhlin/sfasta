use crate::*;

#[derive(Debug, Clone, bincode::Encode, bincode::Decode, Default)]
pub struct Metadata
{
    pub created_by: Option<String>,
    pub citation_doi: Option<String>,
    pub citation_url: Option<String>,
    pub citation_authors: Option<String>,
    pub date_created: u64,
    pub title: Option<String>,
    pub description: Option<String>,
    pub notes: Option<String>,
    pub download_url: Option<String>,
    pub homepage_url: Option<String>,
    pub version: Option<usize>,
}

#[cfg(test)]
mod tests
{
    use super::*;

    #[test]
    pub fn bincode_size_struct()
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        let mut metadata = Metadata::default();

        let encoded_0: Vec<u8> = bincode::encode_to_vec(&metadata, bincode_config).unwrap();
        metadata.created_by = Some("CrateTest".to_string());

        let encoded_1: Vec<u8> = bincode::encode_to_vec(&metadata, bincode_config).unwrap();
        assert!(encoded_0.len() != encoded_1.len());
    }
}
