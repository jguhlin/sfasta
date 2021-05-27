use std::io::{Read, Seek, SeekFrom};

use bincode::Options;

use crate::directory::Directory;
use crate::index::*;
use crate::index_directory::IndexDirectory;
use crate::metadata::Metadata;
use crate::parameters::Parameters;
use crate::*;

const IDX_CHUNK_SIZE: usize = 64 * 1024;
pub struct Sfasta {
    pub version: u64, // I'm going to regret this, but 18,446,744,073,709,551,615 versions should be enough for anybody.
    pub directory: Directory,
    pub parameters: Parameters,
    pub metadata: Metadata,
    pub index_directory: IndexDirectory,
    pub index: Option<Index64>,
    buf: Option<Box<dyn ReadAndSeek>>,
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
            buf: None,
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

    pub fn decompress_all_ids(&mut self) {
        assert!(
            self.buf.is_some(),
            "Sfasta buffer not yet present -- Are you creating a file?"
        );
        let len = self.index.as_ref().unwrap().len();
        let blocks = (len as f64 / 8192_f64).ceil() as usize;

        self.buf
            .as_deref_mut()
            .unwrap()
            .seek(SeekFrom::Start(self.directory.ids_loc))
            .expect("Unable to work with seek API");

        let mut ids: Vec<String> = Vec::with_capacity(len as usize);

        for _ in 0..blocks {
            let compressed: Vec<u8>;
            compressed = bincode::deserialize_from(self.buf.as_deref_mut().unwrap()).unwrap();
            let mut decompressed = lz4_flex::frame::FrameDecoder::new(&compressed[..]);
            let chunk_ids: Vec<String>;
            chunk_ids = bincode::deserialize_from(&mut decompressed).unwrap();
            ids.extend(chunk_ids);
        }

        self.index.as_mut().unwrap().set_ids(ids);
    }

    pub fn find(&mut self, x: &str) -> Result<Option<Vec<String>>, &str> {
        let possibilities = self.index.as_ref().unwrap().find(x);
        if possibilities.is_some() {
            let possibilities = possibilities.unwrap();

            let mut matches = Vec::with_capacity(possibilities.len());

            if self.index.as_ref().unwrap().ids.is_some() {
                // Index is already decompressed, just search it appropriately...

                let idx_ref = self.index.as_ref().unwrap().ids.as_ref().unwrap();

                for loc in possibilities {
                    if idx_ref[loc as usize] == x {
                        matches.push(idx_ref[loc as usize].clone());
                    }
                }
            } else {
                for loc in possibilities {
                    let block = loc as usize / IDX_CHUNK_SIZE;

                    self.buf
                        .as_deref_mut()
                        .unwrap()
                        .seek(SeekFrom::Start(self.directory.ids_loc))
                        .expect("Unable to work with seek API");

                    let mut cur: isize = -1;
                    let mut compressed: Vec<u8> = Vec::new();
                    while cur < block as isize {
                        cur += 1;
                        compressed =
                            bincode::deserialize_from(self.buf.as_deref_mut().unwrap()).unwrap();
                    }

                    let mut decompressed = lz4_flex::frame::FrameDecoder::new(&compressed[..]);
                    let ids: Vec<String> = bincode::deserialize_from(&mut decompressed).unwrap();

                    if ids[loc as usize % IDX_CHUNK_SIZE] == x {
                        matches.push(ids[loc as usize].clone());
                    }
                }
            }

            let return_val;
            if matches.len() > 0 {
                return_val = Some(matches);
            } else {
                return_val = None;
            }

            Ok(return_val)
        } else {
            Ok(None)
        }
    }

    pub fn index_len(&self) -> usize {
        self.index.as_ref().unwrap().len() as usize
    }
}

pub struct SfastaParser {
    pub sfasta: Sfasta,
}

impl SfastaParser {
    pub fn open_from_buffer<R>(mut in_buf: R) -> Sfasta
    where
        R: 'static + Read + Seek + Send,
    {
        let bincode = bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .allow_trailing_bytes();

        let mut sfasta_marker: [u8; 6] = [0; 6];
        in_buf
            .read_exact(&mut sfasta_marker)
            .expect("Unable to read SFASTA Marker");
        assert!(sfasta_marker == "sfasta".as_bytes());

        let mut sfasta = Sfasta::default();

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

        let index_compressed: Vec<u8> = bincode
            .deserialize_from(&mut in_buf)
            .expect("Unable to parse index");

        let mut decompressor = lz4_flex::frame::FrameDecoder::new(&index_compressed[..]);
        let mut index_bincoded = Vec::with_capacity(32 * 1024 * 1024);
        decompressor
            .read_to_end(&mut index_bincoded)
            .expect("Unable to parse index");

        sfasta.index = Some(
            bincode
                .deserialize_from(&index_bincoded[..])
                .expect("Unable to parse index"),
        );

        // let mut parser = SfastaParser { sfasta, in_buf };

        // TODO: Disabled for testing...
        // If there are few enough IDs, let's decompress it and store it in the index...
        // if parser.sfasta.index.as_ref().unwrap().len() <= 8192 * 2 {
        //    parser.decompress_all_ids();
        // }

        sfasta.buf = Some(Box::new(in_buf));

        sfasta
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::BufReader;
    use std::io::Cursor;

    #[test]
    pub fn test_sfasta_find() {
        let mut out_buf = Cursor::new(Vec::new());

        let in_buf = BufReader::new(
            File::open("test_data/test_convert.fasta").expect("Unable to open testing file"),
        );

        crate::conversion::convert_fasta(
            in_buf,
            &mut out_buf,
            8 * 1024,
            6,
            10,
            CompressionType::ZSTD,
            true,
        );

        match out_buf.seek(SeekFrom::Start(0)) {
            Err(x) => panic!("Unable to seek to start of file, {:#?}", x),
            Ok(_) => (),
        };

        let mut sfasta = SfastaParser::open_from_buffer(out_buf);
        assert!(sfasta.index_len() == 3001);

        let output = sfasta.find("does-not-exist");
        assert!(output == Ok(None));

        let output = &sfasta.find("needle").unwrap().unwrap()[0];
        assert!(output == "needle");

        let output = &sfasta.find("needle_last").unwrap().unwrap()[0];
        assert!(output == "needle_last");
    }
}
