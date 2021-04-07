use crate::directory::Directory;
use crate::metadata::Metadata;
use crate::parameters::Parameters;
use crate::*;

pub struct Sfasta {
    pub directory: Directory,
    pub parameters: Parameters,
    pub metadata: Metadata,
}

impl Default for Sfasta {
    fn default() -> Self {
        Sfasta {
            directory: Directory::default(),
            parameters: Parameters::default(),
            metadata: Metadata::default(),
        }
    }
}

impl Sfasta {
    pub fn with_scores(mut self) -> Self {
        self.directory = self.directory.with_scores();
        self
    }

    pub fn block_size(mut self, block_size: u32) -> Self {
        self.parameters.block_size = block_size;
        self
    }

    // TODO: Does nothing right now...
    pub fn compression_type(mut self, compression: CompressionType) -> Self {
        self.parameters.compression_type = compression;
        self
    }
}
