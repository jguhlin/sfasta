use crate::directory::Directory;
use crate::index_directory::IndexDirectory;
use crate::metadata::Metadata;
use crate::parameters::Parameters;
use crate::*;

pub struct Sfasta {
    pub directory: Directory,
    pub parameters: Parameters,
    pub metadata: Metadata,
    pub index_directory: IndexDirectory,
}

impl Default for Sfasta {
    fn default() -> Self {
        Sfasta {
            directory: Directory::default(),
            parameters: Parameters::default(),
            metadata: Metadata::default(),
            index_directory: IndexDirectory::default().with_blocks().with_ids(),
        }
    }
}

impl Sfasta {
    pub fn with_sequences(mut self) -> Self {
        self.directory = self.directory.with_sequences();
        self
    }

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
