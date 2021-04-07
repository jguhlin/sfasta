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
}
