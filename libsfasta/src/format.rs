use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Cursor};

use bincode::Options;

use crate::directory::Directory;
use crate::index_directory::IndexDirectory;
use crate::metadata::Metadata;
use crate::parameters::Parameters;
use crate::index::Index64;
use crate::*;

pub struct Sfasta {
    pub version: u64, // I'm going to regret this, but 18,446,744,073,709,551,615 versions should be enough for anybody.
    pub directory: Directory,
    pub parameters: Parameters,
    pub metadata: Metadata,
    pub index_directory: IndexDirectory,
    pub index: Option<Index64>,
}

impl Default for Sfasta {
    fn default() -> Self {
        Sfasta {
            version: 1,
            directory: Directory::default(),
            parameters: Parameters::default(),
            metadata: Metadata::default(),
            index_directory: IndexDirectory::default().with_blocks().with_ids(),
            index: None,
        }
    }
}

impl Sfasta {

    pub fn open_from_buffer<R: 'static>(mut in_buf: R) -> Self 
    where R: Read + Seek + Send, // + std::convert::AsRef<[u8]>,
    {
        let bincode = bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .allow_trailing_bytes();

        // let mut in_buf = Cursor::new(in_buf);

        let mut sfasta_marker: [u8; 6] = [0; 6];
        in_buf.read_exact(&mut sfasta_marker).expect("Unable to read SFASTA Marker");
        assert!(sfasta_marker == "sfasta".as_bytes());

        let mut sfasta = Sfasta::default();

        sfasta.version = match bincode::deserialize_from(&mut in_buf) {
            Ok(x) => x,
            Err(y) => panic!("Error reading SFASTA directory: {}", y)
        };

        assert!(sfasta.version <= 1); // 1 is the maximum version supported at this stage...

        // TODO: In the future, with different versions, we will need to do different things
        // when we inevitabily introduce incompatabilities...

        sfasta.directory = match bincode::deserialize_from(&mut in_buf) {
            Ok(x) => x,
            Err(y) => panic!("Error reading SFASTA directory: {}", y)
        };

        sfasta.parameters = match bincode::deserialize_from(&mut in_buf) {
            Ok(x) => x,
            Err(y) => panic!("Error reading SFASTA parameters: {}", y)
        };

        sfasta.metadata = match bincode::deserialize_from(&mut in_buf) {
            Ok(x) => x,
            Err(y) => panic!("Error reading SFASTA parameters: {}", y)
        };

        // Next are the sequence blocks, which aren't important right now...
        // The index is much more important to us...

        in_buf
            .seek(SeekFrom::Start(sfasta.directory.index_loc))
            .expect("Unable to work with seek API");

        sfasta.index = Some(bincode.deserialize_from(&mut in_buf).expect("Unable to parse index"));

        sfasta
    }

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
