// Easy, high-performance conversion functions
use std::fs::{metadata, File};
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom};
use std::sync::atomic::Ordering;
use std::thread;

use bincode::Options;

use crate::compression_stream_buffer::CompressionStreamBuffer;
use crate::fasta::*;
use crate::format::{Sfasta, IDX_CHUNK_SIZE, SEQLOCS_CHUNK_SIZE};
use crate::index::IDIndexer;
use crate::structs::WriteAndSeek;
use crate::types::*;
use crate::CompressionType;

pub struct Converter {
    masking: bool,
    index: bool,
    threads: usize,
    block_size: usize,
    index_chunk_size: usize,
    seqlocs_chunk_size: usize,
    quality_scores: bool,
    compression_type: CompressionType,
    index_compression_type: CompressionType,
}

impl Default for Converter {
    fn default() -> Self {
        Converter {
            threads: 8,
            block_size: 4 * 1024 * 1024,   // 4Mb
            index_chunk_size: 64 * 1024,   // 64k
            seqlocs_chunk_size: 64 * 1024, // 64k
            index_compression_type: CompressionType::LZ4,
            index: true,
            masking: false,
            quality_scores: false,
            compression_type: CompressionType::ZSTD,
        }
    }
}

impl Converter {
    // Builder configuration functions...
    pub fn with_masking(mut self) -> Self {
        self.masking = true;
        self
    }

    pub fn with_index(mut self) -> Self {
        self.index = true;
        self
    }

    pub fn without_index(mut self) -> Self {
        self.index = false;
        self
    }

    pub fn with_scores(mut self) -> Self {
        self.quality_scores = true;
        self
    }

    pub fn with_threads(mut self, threads: usize) -> Self {
        assert!(
            threads < u16::MAX as usize,
            "Maximum number of supported threads is u16::MAX"
        );
        self.threads = threads;
        self
    }

    pub fn with_block_size(mut self, block_size: usize) -> Self {
        assert!(
            block_size < u32::MAX as usize,
            "Block size must be less than u32::MAX (~4Gb)"
        );

        self.block_size = block_size;
        self
    }

    pub fn with_index_chunk_size(mut self, chunk_size: usize) -> Self {
        assert!(
            chunk_size < u32::MAX as usize,
            "Chunk size must be less than u32::MAX (~4Gb)"
        );

        self.index_chunk_size = chunk_size;
        self
    }

    pub fn with_seqlocs_chunk_size(mut self, chunk_size: usize) -> Self {
        assert!(
            chunk_size < u32::MAX as usize,
            "Chunk size must be less than u32::MAX (~4Gb)"
        );

        self.seqlocs_chunk_size = chunk_size;
        self
    }

    // Functions to do conversions
    pub fn convert_fasta<W, R: 'static>(self, in_buf: R, out_buf: &mut W, entry_count: usize)
    where
        W: WriteAndSeek,
        R: BufRead + Read + Send,
    {
        assert!(
            entry_count <= u32::MAX as usize,
            "Maximum number of sequences to be stored is limited to u32::MAX, Please e-mail Joseph and I will fix this for you"
        );

        let bincode = bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .allow_trailing_bytes();

        let mut sfasta = Sfasta::default().block_size(self.block_size as u32);

        if self.masking {
            sfasta = sfasta.with_masking();
        }

        sfasta.parameters.compression_type = self.compression_type;
        sfasta.parameters.index_compression_type = self.index_compression_type;
        sfasta.parameters.index_chunk_size = self.index_chunk_size as u32;
        sfasta.parameters.seqlocs_chunk_size = self.seqlocs_chunk_size as u32;

        // Output file
        let mut out_fh = out_buf;

        out_fh
            .write_all("sfasta".as_bytes())
            .expect("Unable to write 'sfasta' to output");

        // Write the directory, parameters, and metadata structs out...
        bincode
            .serialize_into(&mut out_fh, &sfasta.version)
            .expect("Unable to write directory to file");

        let directory_location = out_fh
            .seek(SeekFrom::Current(0))
            .expect("Unable to work with seek API");

        bincode
            .serialize_into(&mut out_fh, &sfasta.directory)
            .expect("Unable to write directory to file");

        bincode
            .serialize_into(&mut out_fh, &sfasta.parameters)
            .expect("Unable to write Parameters to file");

        bincode
            .serialize_into(&mut out_fh, &sfasta.metadata)
            .expect("Unable to write Metadata to file");

        let mut sb = CompressionStreamBuffer::default()
            .with_block_size(self.block_size as u32)
            .with_compression_type(self.compression_type)
            .with_threads(self.threads as u16); // Effectively # of compression threads

        let oq = sb.output_queue();
        let shutdown = sb.shutdown_notified();

        // Pop to a new thread that pushes FASTA sequences into the sequence buffer...
        let reader_handle = thread::spawn(move || {
            let fasta = Fasta::from_buffer(in_buf);

            let mut seq_locs = Vec::with_capacity(512 * 1024);
            for i in fasta {
                let loc = sb.add_sequence(&i.seq[..].to_ascii_uppercase()).unwrap();
                seq_locs.push((i.id, loc));
            }

            // Finalize pushes the last block, which is likely smaller than the complete block size
            match sb.finalize() {
                Ok(()) => (),
                Err(x) => panic!("Unable to finalize sequence buffer, {:#?}", x),
            };

            seq_locs
        });

        let mut block_locs = Vec::with_capacity(512 * 1024);
        let mut pos = out_fh
            .seek(SeekFrom::Current(0))
            .expect("Unable to work with seek API");

        let mut result;

        // For each entry processed by the sequence buffer, pop it out, write the block, and write the block id
        loop {
            result = oq.pop();

            match result {
                None => {
                    if oq.len() == 0 && shutdown.load(Ordering::Relaxed) {
                        break;
                    }
                }
                Some((block_id, sb)) => {
                    bincode::serialize_into(&mut out_fh, &sb)
                        .expect("Unable to write to bincode output");

                    block_locs.push((block_id, pos));

                    pos = out_fh
                        .seek(SeekFrom::Current(0))
                        .expect("Unable to work with seek API");
                }
            }
        }

        // TODO: Here is where we would write out the scores...
        // ... but this fn is only for FASTA right now...

        // TODO: Here is where we would write out the Seqinfo stream (if it's decided to do it)

        // Write out the sequence locs...
        let seq_locs: Vec<(String, Vec<Loc>)> = reader_handle.join().unwrap();

        // TODO: Support for Index32 (and even smaller! What if only 1 or 2 sequences?)
        let mut indexer = crate::index::Index64::with_capacity(entry_count);

        let seqlocs_loc = out_fh
            .seek(SeekFrom::Current(0))
            .expect("Unable to work with seek API");

        sfasta.directory.seqlocs_loc = seqlocs_loc;

        let mut out_fh = BufWriter::with_capacity(8 * 1024 * 1024, &mut out_fh);

        let mut seqlocs_blocks_locs: Vec<u64> =
            Vec::with_capacity(seq_locs.len() / SEQLOCS_CHUNK_SIZE);

        let mut compressed: Vec<u8> = Vec::with_capacity(8 * 1024 * 1024);
        for s in seq_locs
            .iter()
            .enumerate()
            .collect::<Vec<(usize, &(String, Vec<Loc>))>>()
            .chunks(SEQLOCS_CHUNK_SIZE)
        {
            if self.index {
                for (i, (id, _)) in s {
                    indexer.add(&id, *i as u32).expect("Unable to add to index");
                }
            }

            seqlocs_blocks_locs.push(
                out_fh
                    .seek(SeekFrom::Current(0))
                    .expect("Unable to work with seek API"),
            );

            let locs: Vec<_> = s.iter().map(|(_, (_, l))| l).collect();

            let mut compressor = lz4_flex::frame::FrameEncoder::new(compressed);

            bincode::serialize_into(&mut compressor, &locs)
                .expect("Unable to bincode locs into compressor");
            compressed = compressor.finish().unwrap();

            bincode::serialize_into(&mut out_fh, &compressed)
                .expect("Unable to write Metadata to file");

            compressed.clear();
        }

        if self.index {
            let mut indexer = indexer.finalize();

            // ID Index
            let id_index_pos = out_fh
                .seek(SeekFrom::Current(0))
                .expect("Unable to work with seek API");

            let ids = indexer.ids.take().unwrap();

            let compressed: Vec<u8> = Vec::with_capacity(8 * 1024 * 1024);
            let mut compressor = lz4_flex::frame::FrameEncoder::new(compressed);

            bincode::serialize_into(&mut compressor, &indexer)
                .expect("Unable to bincode index to compressor");

            let compressed = compressor.finish().unwrap();

            bincode::serialize_into(&mut out_fh, &compressed)
                .expect("Unable to bincode compressed index");

            // bincode::serialize_into(&mut out_fh, &indexer).expect("Unable to write directory to file");
            let ids_loc = out_fh
                .seek(SeekFrom::Current(0))
                .expect("Unable to work with seek API");

            let mut ids_blocks_locs: Vec<u64> = Vec::with_capacity(ids.len() / IDX_CHUNK_SIZE);

            // Write out the IDs vector...
            for chunk in ids.chunks(IDX_CHUNK_SIZE) {
                let pos = out_fh
                    .seek(SeekFrom::Current(0))
                    .expect("Unable to work with seek API");
                ids_blocks_locs.push(pos);

                let output: Vec<u8> = Vec::with_capacity(4 * 1024 * 24);
                let mut compressor = lz4_flex::frame::FrameEncoder::new(output);
                bincode::serialize_into(&mut compressor, &chunk)
                    .expect("Unable to write directory to file");

                let compressed = compressor.finish().expect("Unable to compress ID stream");

                bincode::serialize_into(&mut out_fh, &compressed)
                    .expect("Unable to write directory to file");
            }

            // ID Block Locs

            let id_blocks_index_loc = out_fh
                .seek(SeekFrom::Current(0))
                .expect("Unable to work with seek API");

            let ids_blocks_locs: Vec<u64> =
                ids_blocks_locs.into_iter().map(|x| x - ids_loc).collect();

            let output: Vec<u8> = Vec::with_capacity(4 * 1024 * 24);
            let mut compressor = lz4_flex::frame::FrameEncoder::new(output);
            bincode::serialize_into(&mut compressor, &ids_blocks_locs)
                .expect("Unable to write directory to file");

            let compressed = compressor.finish().expect("Unable to compress ID stream");

            bincode::serialize_into(&mut out_fh, &compressed)
                .expect("Unable to write directory to file");

            // SeqLoc Blocks Locs
            let seqloc_blocks_index_loc = out_fh
                .seek(SeekFrom::Current(0))
                .expect("Unable to work with seek API");

            let seqlocs_blocks_locs: Vec<u64> = seqlocs_blocks_locs
                .into_iter()
                .map(|x| x - seqlocs_loc)
                .collect();

            let output: Vec<u8> = Vec::with_capacity(4 * 1024 * 24);
            let mut compressor = lz4_flex::frame::FrameEncoder::new(output);
            bincode::serialize_into(&mut compressor, &seqlocs_blocks_locs)
                .expect("Unable to write directory to file");

            let compressed = compressor.finish().expect("Unable to compress ID stream");

            bincode::serialize_into(&mut out_fh, &compressed)
                .expect("Unable to write directory to file");

            sfasta.directory.seqloc_blocks_index_loc = Some(seqloc_blocks_index_loc);
            sfasta.directory.id_blocks_index_loc = Some(id_blocks_index_loc);
            sfasta.directory.index_loc = Some(id_index_pos);
            sfasta.directory.ids_loc = ids_loc;
        }

        // Block Index
        let block_index_pos = out_fh
            .seek(SeekFrom::Current(0))
            .expect("Unable to work with seek API");

        let block_locs: Vec<u64> = block_locs.iter().map(|x| x.1).collect();

        let output: Vec<u8> = Vec::with_capacity(4 * 1024 * 24);

        let mut compressor = lz4_flex::frame::FrameEncoder::new(output);
        bincode::serialize_into(&mut compressor, &block_locs)
            .expect("Unable to write block locs to file");

        let compressed = compressor
            .finish()
            .expect("Unable to compress Block Locs stream");

        bincode::serialize_into(&mut out_fh, &compressed)
            .expect("Unable to write directory to file");

        //bincode::serialize_into(&mut out_fh, &block_locs).expect("Unable to write directory to file");

        // TODO: Scores Block Index

        // TODO: Masking Block

        // Go to the beginning, and write the location of the index

        sfasta.directory.block_index_loc = block_index_pos;

        out_fh
            .seek(SeekFrom::Start(directory_location))
            .expect("Unable to rewind to start of the file");

        // Here we re-write the directory information at the start of the file, allowing for
        // easy hops to important areas while keeping everything in a single file
        bincode::serialize_into(&mut out_fh, &sfasta.directory)
            .expect("Unable to write directory to file");
    }
}

// TODO: Add support for metadata here...
// TODO: Will likely need to be the same builder style
// TODO: Will need to generalize this function so it works with FASTA & FASTQ & Masking

pub fn generic_open_file(filename: &str) -> (usize, bool, Box<dyn Read + Send>) {
    let filesize = metadata(filename)
        .unwrap_or_else(|_| panic!("{}", &format!("Unable to open file: {}", filename)))
        .len();

    let file = match File::open(filename) {
        Err(why) => panic!("Couldn't open {}: {}", filename, why.to_string()),
        Ok(file) => file,
    };

    let file = BufReader::new(file);
    let mut compressed: bool = false;

    let fasta: Box<dyn Read + Send> = if filename.ends_with("gz") {
        compressed = true;
        Box::new(flate2::read::GzDecoder::new(file))
    } else if filename.ends_with("snappy") || filename.ends_with("sz") || filename.ends_with("sfai")
    {
        compressed = true;
        Box::new(snap::read::FrameDecoder::new(file))
    } else {
        Box::new(file)
    };

    (filesize as usize, compressed, fasta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directory::Directory;
    use crate::metadata::Metadata;
    use crate::parameters::Parameters;
    use crate::sequence_block::SequenceBlockCompressed;
    use std::io::Cursor;

    #[test]
    pub fn test_create_sfasta() {
        let bincode = bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .allow_trailing_bytes();

        let mut out_buf = Cursor::new(Vec::new());

        let in_buf = BufReader::new(
            File::open("test_data/test_convert.fasta").expect("Unable to open testing file"),
        );

        let converter = Converter::default().with_threads(6).with_block_size(8192);

        converter.convert_fasta(in_buf, &mut out_buf, 10);

        match out_buf.seek(SeekFrom::Start(0)) {
            Err(x) => panic!("Unable to seek to start of file, {:#?}", x),
            Ok(_) => (),
        };

        let mut sfasta_marker: [u8; 6] = [0; 6];
        out_buf
            .read_exact(&mut sfasta_marker)
            .expect("Unable to read SFASTA Marker");
        assert!(sfasta_marker == "sfasta".as_bytes());

        let _version: u64 = bincode.deserialize_from(&mut out_buf).unwrap();
        let _directory: Directory = bincode.deserialize_from(&mut out_buf).unwrap();
        println!("{:#?}", _directory);

        let _parameters: Parameters = bincode.deserialize_from(&mut out_buf).unwrap();
        let _metadata: Metadata = bincode.deserialize_from(&mut out_buf).unwrap();

        let b: SequenceBlockCompressed = bincode::deserialize_from(&mut out_buf).unwrap();
        let b = b.decompress(CompressionType::ZSTD);

        assert!(b.len() == 8192);
    }
}
