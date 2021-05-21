use std::io::{Read, Seek, SeekFrom};

use bincode::Options;

use crate::directory::Directory;
use crate::index::*;
use crate::index_directory::IndexDirectory;
use crate::metadata::Metadata;
use crate::parameters::Parameters;
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

pub struct SfastaParser<R: 'static + Read + Seek + Send> {
    pub sfasta: Sfasta,
    in_buf: R,
}

impl<R: 'static + Read + Seek + Send> SfastaParser<R> {
    pub fn open_from_buffer(mut in_buf: R) -> Self {
        let bincode = bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .allow_trailing_bytes();

        let mut sfasta_marker: [u8; 6] = [0; 6];
        in_buf
            .read_exact(&mut sfasta_marker)
            .expect("Unable to read SFASTA Marker");
        assert!(sfasta_marker == "sfasta".as_bytes());

        let mut sfasta = Sfasta::default();

        println!("Got header, reading directory and metadata...");

        sfasta.version = match bincode::deserialize_from(&mut in_buf) {
            Ok(x) => x,
            Err(y) => panic!("Error reading SFASTA directory: {}", y),
        };

        assert!(sfasta.version <= 1); // 1 is the maximum version supported at this stage...

        // TODO: In the future, with different versions, we will need to do different things
        // when we inevitabily introduce incompatabilities...

        sfasta.directory = match bincode::deserialize_from(&mut in_buf) {
            Ok(x) => x,
            Err(y) => panic!("Error reading SFASTA directory: {}", y),
        };

        sfasta.parameters = match bincode::deserialize_from(&mut in_buf) {
            Ok(x) => x,
            Err(y) => panic!("Error reading SFASTA parameters: {}", y),
        };

        sfasta.metadata = match bincode::deserialize_from(&mut in_buf) {
            Ok(x) => x,
            Err(y) => panic!("Error reading SFASTA parameters: {}", y),
        };

        // Next are the sequence blocks, which aren't important right now...
        // The index is much more important to us...

        in_buf
            .seek(SeekFrom::Start(sfasta.directory.index_loc))
            .expect("Unable to work with seek API");

        println!("Reading index...");

        let index_compressed: Vec<u8> = bincode
            .deserialize_from(&mut in_buf)
            .expect("Unable to parse index");

        println!("Got compressed index, decompressing...");

        let mut decompressor = lz4_flex::frame::FrameDecoder::new(&index_compressed[..]);
        let mut index_bincoded = Vec::with_capacity(32 * 1024 * 1024);
        decompressor
            .read_to_end(&mut index_bincoded)
            .expect("Unable to parse index");

        println!("Parsing index");

        sfasta.index = Some(
            bincode
                .deserialize_from(&index_bincoded[..])
                .expect("Unable to parse index"),
        );

        println!("Index loaded");

        let mut parser = SfastaParser { sfasta, in_buf };

        // If there are few enough IDs, let's decompress it and store it in the index...
        if parser.sfasta.index.as_ref().unwrap().len() <= 8192 * 2 {
            parser.decompress_all_ids();
        }

        parser
    }

    pub fn decompress_all_ids(&mut self) {
        let len = self.sfasta.index.as_ref().unwrap().len();
        let blocks = (len as f64 / 8192_f64).ceil() as usize;

        self.in_buf
            .seek(SeekFrom::Start(self.sfasta.directory.ids_loc))
            .expect("Unable to work with seek API");

        let mut ids: Vec<String> = Vec::with_capacity(len as usize);

        for _ in 0..blocks {
            let compressed: Vec<u8>;
            compressed = bincode::deserialize_from(&mut self.in_buf).unwrap();
            let mut decompressed = lz4_flex::frame::FrameDecoder::new(&compressed[..]);
            let chunk_ids: Vec<String>;
            chunk_ids = bincode::deserialize_from(&mut decompressed).unwrap();
            ids.extend(chunk_ids);
        }

        self.sfasta.index.as_mut().unwrap().set_ids(ids);
    }
}
