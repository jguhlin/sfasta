// Easy, high-performance conversion functions
use crossbeam::thread;
use log;
use std::fs::{metadata, File};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::num::NonZeroU64;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::compression_stream_buffer::CompressionStreamBuffer;
use crate::data_types::*;
use crate::dual_level_index::*;
use crate::fasta::*;
use crate::format::DirectoryOnDisk;
use crate::format::Sfasta;
use crate::structs::WriteAndSeek;
use crate::CompressionType;
use crate::utils::*;

pub struct Converter {
    masking: bool,
    index: bool,
    threads: usize,
    block_size: usize,
    seqlocs_chunk_size: usize,
    quality_scores: bool,
    compression_type: CompressionType,
    dict: Option<Vec<u8>>,
}

impl Default for Converter {
    fn default() -> Self {
        Converter {
            threads: 8,
            block_size: 16 * 1024 * 1024,    // 16Mb
            seqlocs_chunk_size: 256 * 1024,  // 256k
            index: true,
            masking: false,
            quality_scores: false,
            compression_type: CompressionType::ZSTD,
            dict: None,
        }
    }
}

impl Converter {
    // Builder configuration functions...
    pub fn with_dict(mut self, dict: Vec<u8>) -> Self {
        self.dict = Some(dict);
        self
    }

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

    pub fn with_seqlocs_chunk_size(mut self, chunk_size: usize) -> Self {
        assert!(
            chunk_size < u32::MAX as usize,
            "Chunk size must be less than u32::MAX (~4Gb)"
        );

        self.seqlocs_chunk_size = chunk_size;

        self
    }

    pub fn with_compression_type(mut self, ct: CompressionType) -> Self {
        self.compression_type = ct;
        self
    }

    pub fn write_headers<W>(&self, mut out_fh: &mut W, sfasta: &Sfasta) -> u64
    where
        W: Write + Seek,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        out_fh
            .write_all("sfasta".as_bytes())
            .expect("Unable to write 'sfasta' to output");

        // Write the directory, parameters, and metadata structs out...

        // Write the version
        bincode::encode_into_std_write(&sfasta.version, &mut out_fh, bincode_config)
            .expect("Unable to write directory to file");

        // Write the directory
        let directory_location = out_fh
            .seek(SeekFrom::Current(0))
            .expect("Unable to work with seek API");

        let dir: DirectoryOnDisk = sfasta.directory.clone().into();
        bincode::encode_into_std_write(dir, &mut out_fh, bincode_config)
            .expect("Unable to write directory to file");

        // Write the parameters
        bincode::encode_into_std_write(&sfasta.parameters, &mut out_fh, bincode_config)
            .expect("Unable to write Parameters to file");

        // Write the metadata
        bincode::encode_into_std_write(&sfasta.metadata, &mut out_fh, bincode_config)
            .expect("Unable to write Metadata to file");

        // Return the directory location
        directory_location
    }

    /// Main conversion function
    pub fn convert_fasta<'convert, W, R>(self, mut in_buf: R, mut out_fh: W) -> W
    where
        W: WriteAndSeek + 'convert + std::fmt::Debug,
        R: Read + Send + 'convert,
    {
        let mut debug_size: Vec<(String, usize)> = Vec::new();

        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        let mut sfasta = Sfasta::default().block_size(self.block_size as u32);

        // Store masks as series of 0s and 1s... Vec<bool>
        // TODO
        if self.masking {
            sfasta = sfasta.with_masking();
        }

        sfasta.parameters.compression_type = self.compression_type;
        sfasta.parameters.seqlocs_chunk_size = self.seqlocs_chunk_size as u32;

        // Set dummy values for the directory
        sfasta.directory.dummy();

        // Store the location of the directory so we can update it later...
        // It's right after the SFASTA and version identifier...
        let directory_location = self.write_headers(&mut out_fh, &sfasta);

        // Put everything in Arcs and Mutexes to allow for multithreading
        // let in_buf = Arc::new(Mutex::new(in_buf));
        // let out_buf = Arc::new(Mutex::new(out_buf));

        log::debug!(
            "Writing sequences start... {}",
            out_fh.seek(SeekFrom::Current(0)).unwrap()
        );

        let start = out_fh.seek(SeekFrom::Current(0)).unwrap();

        // Write sequences
        let sb = CompressionStreamBuffer::default()
            .with_block_size(self.block_size as u32)
            .with_compression_type(self.compression_type)
            .with_threads(self.threads as u16); // Effectively # of compression threads

        // Function returns:
        // Vec<(String, Location)>
        // block_index_pos
        let (seq_locs, block_index_pos) = write_fasta_sequence(sb, &mut in_buf, &mut out_fh);

        let end = out_fh.seek(SeekFrom::Current(0)).unwrap();

        debug_size.push(("sequences".to_string(), (end - start) as usize));

        log::debug!(
            "Writing sequences finished... {}",
            out_fh.seek(SeekFrom::Current(0)).unwrap()
        );

        // TODO: Here is where we would write out the scores...
        // ... but this fn is only for FASTA right now...

        // TODO: Here is where we would write out the masking...

        // TODO: Here is where we would write out the Seqinfo stream (if it's decided to do it)

        // TODO: Support for Index32 (and even smaller! What if only 1 or 2 sequences?)
        let mut indexer = crate::dual_level_index::DualIndexBuilder::with_capacity(seq_locs.len());

        let mut out_buf = BufWriter::with_capacity(128 * 1024, out_fh);

        let mut seqlocs = SeqLocs::with_data(seq_locs);

        // The index points to the location of the Location structs.
        // Location blocks are chunked into SEQLOCS_CHUNK_SIZE
        // So index.get(id) --> integer position of the location struct
        // Which will be in integer position / SEQLOCS_CHUNK_SIZE chunk offset
        // This will then point to the different location blocks where the sequence is...
        //
        // TODO: Optional index can probably be handled better...

        let to_index = seqlocs.data.as_ref().unwrap().clone();

        let mut id_index_pos = 0;

        let mut seqlocs_location = 0;

        thread::scope(|s| {
            // Start a thread to build the index...
            let index_handle = Some(s.spawn(|_| {
                for (i, seqloc) in to_index.iter().enumerate() {
                    indexer.add(seqloc.id.clone(), i as u32);
                }

                let indexer: DualIndexWriter = indexer.into();
                indexer
            }));

            // Use the main thread to write the sequence locations...
            log::debug!(
                "Writing SeqLocs to file. {}",
                out_buf.seek(SeekFrom::Current(0)).unwrap()
            );

            let start = out_buf.seek(SeekFrom::Current(0)).unwrap();

            seqlocs_location = seqlocs.write_to_buffer(&mut out_buf);
            log::debug!(
                "Writing SeqLocs to file: COMPLETE. {}",
                out_buf.seek(SeekFrom::Current(0)).unwrap()
            );

            let end = out_buf.seek(SeekFrom::Current(0)).unwrap();
            debug_size.push(("seqlocs".to_string(), (end - start) as usize));

            if self.index {
                id_index_pos = out_buf
                    .seek(SeekFrom::Current(0))
                    .expect("Unable to work with seek API");

                let mut index = index_handle.unwrap().join().unwrap();
                log::debug!(
                    "Writing index to file. {}",
                    out_buf.seek(SeekFrom::Current(0)).unwrap()
                );

                let start = out_buf.seek(SeekFrom::Current(0)).unwrap();

                index.write_to_buffer(&mut out_buf);

                let end = out_buf.seek(SeekFrom::Current(0)).unwrap();
                debug_size.push(("index".to_string(), (end - start) as usize));

                log::debug!(
                    "Writing index to file: COMPLETE. {}",
                    out_buf.seek(SeekFrom::Current(0)).unwrap()
                );
            }
        })
        .expect("Error");

        sfasta.directory.seqlocs_loc = NonZeroU64::new(seqlocs_location);
        sfasta.directory.index_loc = NonZeroU64::new(id_index_pos);

        // TODO: Scores Block Index

        // TODO: Masking Block
        // Use bitvec implementation

        // Go to the beginning, and write the location of the index

        sfasta.directory.block_index_loc = NonZeroU64::new(block_index_pos);

        out_buf
            .seek(SeekFrom::Start(directory_location))
            .expect("Unable to rewind to start of the file");

        // Here we re-write the directory information at the start of the file, allowing for
        // easy jumps to important areas while keeping everything in a single file

        let start = out_buf.seek(SeekFrom::Current(0)).unwrap();

        let dir: DirectoryOnDisk = sfasta.directory.clone().into();
        bincode::encode_into_std_write(dir, &mut out_buf, bincode_config)
            .expect("Unable to write directory to file");

        let end = out_buf.seek(SeekFrom::Current(0)).unwrap();
        debug_size.push(("directory".to_string(), (end - start) as usize));

        println!("DEBUG: {:?}", debug_size);

        out_buf.into_inner().unwrap()
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
        Err(why) => panic!("Couldn't open {}: {}", filename, why),
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

/// Input filehandle goes in, output goes out.
/// Function returns:
/// Vec<(String, Location)>
/// block_index_pos
/// in_buffer
/// out_buffer

pub fn write_fasta_sequence<'write, W, R>(
    mut sb: CompressionStreamBuffer,
    in_buf: &mut R,
    mut out_fh: &mut W,
) -> (Vec<SeqLoc>, u64)
where
    W: WriteAndSeek + 'write,
    R: Read + Send + 'write,
{
    let bincode_config = bincode::config::standard().with_fixed_int_encoding();

    // Get a handle for the output queue from the sequence compressor
    let oq = sb.output_queue();

    // Get a handle to the shutdown flag...
    let shutdown = sb.shutdown_notified();

    let mut seq_locs: Option<Vec<SeqLoc>> = None;
    let mut block_index_pos = None;

    // Pop to a new thread that pushes FASTA sequences into the sequence buffer...
    thread::scope(|s| {
        let mut out_buf = BufWriter::with_capacity(128 * 1024, &mut out_fh);
        let reader_handle = s.spawn(move |_| {
            // Convert reader into buffered reader then into the Fasta struct (and iterator)
            let mut in_buf_reader = BufReader::new(in_buf);
            let fasta = Fasta::from_buffer(&mut in_buf_reader);

            // Store the ID of the sequence and the Location (SeqLocs)
            // TODO: Auto adjust based off of size of input FASTA file
            let mut seq_locs = Vec::with_capacity(512 * 1024);

            // For each Sequence in the fasta file, make it upper case (masking is stored separately)
            // Add the sequence, get the SeqLocs and store them in Location struct
            // And store that in seq_locs Vec...
            for mut i in fasta {
                i.seq[..].make_ascii_uppercase();
                let loc = sb.add_sequence(&i.seq[..]).unwrap();
                let mut location = SeqLoc::new(i.id);
                location.sequence = Some(loc);
                seq_locs.push(location);
            }

            // Finalize pushes the last block, which is likely smaller than the complete block size
            match sb.finalize() {
                Ok(()) => (),
                Err(x) => panic!("Unable to finalize sequence buffer, {:#?}", x),
            };

            // Return seq_locs to the main thread
            Some(seq_locs)
        });

        // Store the location of the Sequence Blocks...
        // Stored as Vec<(u32, u64)> because the multithreading means it does not have to be in order
        let mut block_locs = Vec::with_capacity(512 * 1024);
        let mut pos = out_buf
            .seek(SeekFrom::Current(0))
            .expect("Unable to work with seek API");

        let mut result;

        // For each entry processed by the sequence buffer, pop it out, write the block, and write the block id
        // FORMAT: Write each sequence block to file

        // This writes out the sequence blocks (seqblockcompressed)
        let start = out_buf.seek(SeekFrom::Current(0)).unwrap();

        loop {
            result = oq.pop();

            match result {
                None => {
                    if oq.is_empty() && shutdown.load(Ordering::Relaxed) {
                        break;
                    }
                }
                Some((block_id, sb)) => {
                    bincode::encode_into_std_write(&sb, &mut out_buf, bincode_config)
                        .expect("Unable to write to bincode output");
                    // log::debug!("Writer wrote block {}", block_id);

                    block_locs.push((block_id, pos));

                    pos = out_buf
                        .seek(SeekFrom::Current(0))
                        .expect("Unable to work with seek API");
                }
            }
        }

        let end = out_buf.seek(SeekFrom::Current(0)).unwrap();
        println!("DEBUG: Wrote {} bytes of sequence blocks", end - start);

        let start = out_buf.seek(SeekFrom::Current(0)).unwrap();

        // Block Index
        block_index_pos = Some(
            out_buf
                .seek(SeekFrom::Current(0))
                .expect("Unable to work with seek API"),
        );

        // Write the block index to file

        block_locs.sort_by(|a, b| a.0.cmp(&b.0));
        let block_locs: Vec<u64> = block_locs.iter().map(|x| x.1).collect();

        let block_locs_u32 = unsafe {
            std::slice::from_raw_parts(
                block_locs.as_ptr() as *const u32,
                block_locs.len() * std::mem::size_of::<u64>() / std::mem::size_of::<u32>(),
            )
        };

        let (num_bits, bitpacked) = bitpack_u32(block_locs_u32);
        bincode::encode_into_std_write(&num_bits, &mut out_buf, bincode_config)
            .expect("Unable to write to bincode output");
        bincode::encode_into_std_write(&bitpacked, &mut out_buf, bincode_config).unwrap();

        let end = out_buf.seek(SeekFrom::Current(0)).unwrap();
        println!("DEBUG: Wrote {} bytes of block index", end - start);

        // let compressed: Vec<u8> = Vec::with_capacity(4 * 1024 * 1024);
        // let mut encoder = zstd::stream::Encoder::new(compressed, -3).unwrap();
        // bincode::encode_into_std_write(block_locs, &mut encoder, bincode_config)
            //.expect("Unable to write to bincode output");

        //let compressed = encoder.finish().unwrap();

        //bincode::encode_into_std_write(compressed, &mut out_buf, bincode_config)
            // .expect("Unable to write Sequence Blocks to file");
        seq_locs = reader_handle.join().unwrap();
    })
    .expect("Error");

    (seq_locs.expect("Error"), block_index_pos.expect("Error"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::Metadata;
    use crate::parameters::Parameters;
    use crate::sequence_block::SequenceBlockCompressed;
    use std::io::Cursor;

    #[test]
    pub fn test_create_sfasta() {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        let out_buf = Cursor::new(Vec::new());

        let in_buf = BufReader::new(
            File::open("test_data/test_convert.fasta").expect("Unable to open testing file"),
        );

        let converter = Converter::default().with_threads(6).with_block_size(8192);

        let mut out_buf = converter.convert_fasta(in_buf, out_buf);

        if let Err(x) = out_buf.seek(SeekFrom::Start(0)) {
            panic!("Unable to seek to start of file, {:#?}", x)
        };

        let mut sfasta_marker: [u8; 6] = [0; 6];
        out_buf
            .read_exact(&mut sfasta_marker)
            .expect("Unable to read SFASTA Marker");
        assert!(sfasta_marker == "sfasta".as_bytes());

        let _version: u64 = bincode::decode_from_std_read(&mut out_buf, bincode_config).unwrap();

        let directory: DirectoryOnDisk =
            bincode::decode_from_std_read(&mut out_buf, bincode_config).unwrap();

        let dir = directory;
        println!("{:#?}", dir);

        let _parameters: Parameters =
            bincode::decode_from_std_read(&mut out_buf, bincode_config).unwrap();

        let _metadata: Metadata =
            bincode::decode_from_std_read(&mut out_buf, bincode_config).unwrap();

        let b: SequenceBlockCompressed =
            bincode::decode_from_std_read(&mut out_buf, bincode_config).unwrap();
        let b = b.decompress(CompressionType::ZSTD);

        assert!(b.len() == 8192);
    }
}
