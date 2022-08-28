// Easy, high-performance conversion functions
use crossbeam::thread;
use crossbeam::queue::ArrayQueue;
use crossbeam::utils::Backoff;

use std::fs::{metadata, File};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::num::NonZeroU64;
use std::sync::atomic::Ordering;
use std::time::Instant;
use std::sync::Arc;

use crate::compression_stream_buffer::{CompressionStreamBuffer, CompressionStreamBufferConfig};
use crate::data_types::*;
use crate::dual_level_index::*;
use crate::fasta::*;
use crate::format::Sfasta;
use crate::utils::*;
use crate::CompressionType;

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
            block_size: 4 * 1024 * 1024,    // 1Mb
            seqlocs_chunk_size: 256 * 1024, // 256k
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
    // TODO: Switch R and W into borrows
    pub fn convert_fasta<'convert, W, R>(self, mut in_buf: R, mut out_fh: W) -> W
    where
        W: WriteAndSeek + 'convert + std::fmt::Debug,
        R: Read + Send + 'convert,
    {
        let fn_start_time = std::time::Instant::now();

        let mut debug_size: Vec<(String, usize)> = Vec::new();

        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        assert!(self.block_size < u32::MAX as usize);

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
        let sb_config = CompressionStreamBufferConfig::default()
            .with_block_size(self.block_size as u32)
            .with_compression_type(self.compression_type)
            .with_threads(self.threads as u16); // Effectively # of compression threads

        let start_time = std::time::Instant::now();

        // Function returns:
        // Vec<(String, Location)>
        // block_index_pos
        let (ids, seq_locs, block_index_pos, headers_location, ids_location, masking_location) =
            write_fasta_sequence(sb_config, &mut in_buf, &mut out_fh);

        let end_time = std::time::Instant::now();

        log::debug!(
            "Write Fasta Sequence write time: {:?}",
            end_time - start_time
        );

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

        let mut out_buf = BufWriter::new(out_fh);

        let start_time = std::time::Instant::now();

        let mut seqlocs = SeqLocs::with_data(seq_locs);

        let end_time = std::time::Instant::now();

        log::debug!("Create SeqLocs Struct time: {:?}", end_time - start_time);

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
                for (i, id) in ids.into_iter().enumerate() {
                    indexer.add(id, i as u32);
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

            let start_time = std::time::Instant::now();

            seqlocs_location = seqlocs.write_to_buffer(&mut out_buf);
            log::debug!(
                "Writing SeqLocs to file: COMPLETE. {}",
                out_buf.seek(SeekFrom::Current(0)).unwrap()
            );

            let end_time = std::time::Instant::now();
            log::debug!("SeqLocs write time: {:?}", end_time - start_time);

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

                let start_time = std::time::Instant::now();
                index.write_to_buffer(&mut out_buf);
                let end_time = std::time::Instant::now();
                log::debug!("Index write time: {:?}", end_time - start_time);

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
        sfasta.directory.headers_loc = headers_location;
        sfasta.directory.ids_loc = ids_location;
        sfasta.directory.masking_loc = masking_location;

        // TODO: Scores Block Index

        // Go to the beginning, and write the location of the index

        sfasta.directory.block_index_loc = NonZeroU64::new(block_index_pos);

        out_buf
            .seek(SeekFrom::Start(directory_location))
            .expect("Unable to rewind to start of the file");

        // Here we re-write the directory information at the start of the file, allowing for
        // easy jumps to important areas while keeping everything in a single file

        let start = out_buf.seek(SeekFrom::Current(0)).unwrap();

        let start_time = std::time::Instant::now();

        let dir: DirectoryOnDisk = sfasta.directory.clone().into();
        bincode::encode_into_std_write(dir, &mut out_buf, bincode_config)
            .expect("Unable to write directory to file");

        let end = out_buf.seek(SeekFrom::Current(0)).unwrap();
        debug_size.push(("directory".to_string(), (end - start) as usize));

        let end_time = std::time::Instant::now();

        log::debug!("Directory write time: {:?}", end_time - start_time);

        log::debug!("DEBUG: {:?}", debug_size);

        let fn_end_time = std::time::Instant::now();
        log::debug!("Conversion time: {:?}", fn_end_time - fn_start_time);

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

#[derive(Debug)]
enum Work {
    Payload(Sequence),
    Shutdown,
}

impl Work {
    fn is_shutdown(&self) -> bool {
        matches!(self, Work::Shutdown)
    }

    fn payload(self) -> Sequence {
        match self {
            Work::Payload(s) => s,
            _ => panic!("Unable to get payload from shutdown"),
        }
    }
}

pub fn write_fasta_sequence<'write, W, R>(
    sb_config: CompressionStreamBufferConfig,
    in_buf: &mut R,
    mut out_fh: &mut W,
) -> (
    Vec<std::sync::Arc<String>>,
    Vec<SeqLoc>,
    u64,
    Option<NonZeroU64>,
    Option<NonZeroU64>,
    Option<NonZeroU64>,
)
where
    W: WriteAndSeek + 'write,
    R: Read + Send + 'write,
{
    let bincode_config = bincode::config::standard().with_fixed_int_encoding();

    let mut sb = CompressionStreamBuffer::from_config(sb_config);

    // Get a handle for the output queue from the sequence compressor
    let oq = sb.get_output_queue();

    // Get a handle to the shutdown flag...
    let shutdown = sb.get_shutdown_flag();

    let mut seq_locs: Option<Vec<SeqLoc>> = None;
    let mut block_index_pos = None;
    let mut headers = None;
    let mut headers_location = None;
    let mut ids = None;
    let mut ids_location = None;
    let mut ids_string = Vec::new();
    let mut masking = None;
    let mut masking_location = None;

    let fasta_queue: Arc<ArrayQueue<Work>> = std::sync::Arc::new(ArrayQueue::new(8192));

    let fasta_queue_in = Arc::clone(&fasta_queue);
    let fasta_queue_out = Arc::clone(&fasta_queue);

    // Pop to a new thread that pushes FASTA sequences into the sequence buffer...
    thread::scope(|s| {
        let fasta_thread = s.spawn(|_| {
            // Convert reader into buffered reader then into the Fasta struct (and iterator)
            let mut in_buf_reader = BufReader::with_capacity(512 * 1024, in_buf);
            let fasta = Fasta::from_buffer(&mut in_buf_reader);
    
            let backoff = Backoff::new();
    
            for x in fasta {
                let mut d = Work::Payload(x);
                while let Err(z) = fasta_queue_in.push(d) {
                    d = z;
                    backoff.snooze();               
                }
            }

            while fasta_queue_in.push(Work::Shutdown).is_err() {
                backoff.snooze();
            }
            
        });

        let mut out_buf = BufWriter::new(&mut out_fh);
        let reader_handle = s.spawn(move |_| {
            sb.initialize();

            // Store the ID of the sequence and the Location (SeqLocs)
            // TODO: Auto adjust based off of size of input FASTA file
            let mut seq_locs = Vec::with_capacity(1024);

            let mut headers = Headers::default();
            let mut ids = Ids::default();
            let mut ids_string = Vec::new();
            let mut masking = Masking::default();

            // For each Sequence in the fasta file, make it upper case (masking is stored separately)
            // Add the sequence, get the SeqLocs and store them in Location struct
            // And store that in seq_locs Vec...

            let backoff = Backoff::new();

            loop {
                match fasta_queue_out.pop() {
                    Some(Work::Payload(seq)) => {
                        let (seqid, seqheader, mut seq) = seq.into_parts();
                        let mut location = SeqLoc::new();
                        location.masking = masking.add_masking(&seq[..]);
                        let loc = sb.add_sequence(&mut seq[..]).unwrap(); // Destructive, capitalizes everything...
                        let myid = std::sync::Arc::new(seqid);
                        ids_string.push(std::sync::Arc::clone(&myid));
                        let idloc = ids.add_id(std::sync::Arc::clone(&myid));
                        if !seqheader.is_empty() {
                            location.headers = Some(headers.add_header(seqheader));
                        }
                        location.ids = Some(idloc);
                        location.sequence = Some(loc);
                        seq_locs.push(location);
                    },
                    Some(Work::Shutdown) => {
                        break
                    },
                    None => {
                        backoff.snooze();
                    }
                }
            }

            // Finalize pushes the last block, which is likely smaller than the complete block size
            match sb.finalize() {
                Ok(()) => (),
                Err(x) => panic!("Unable to finalize sequence buffer, {:#?}", x),
            };

            // Return seq_locs to the main thread
            (seq_locs, headers, ids, ids_string, masking)
        });

        // Store the location of the Sequence Blocks...
        // Stored as Vec<(u32, u64)> because the multithreading means it does not have to be in order
        let mut block_locs = Vec::with_capacity(2 * 1024 * 1024);
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

        fasta_thread.join().unwrap();

        let end = out_buf.seek(SeekFrom::Current(0)).unwrap();
        log::debug!("DEBUG: Wrote {} bytes of sequence blocks", end - start);

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
        log::debug!("DEBUG: Wrote {} bytes of block index", end - start);

        let j = reader_handle.join().expect("Unable to join thread");
        seq_locs = Some(j.0);
        headers = Some(j.1);
        ids = Some(j.2);
        ids_string = j.3;
        masking = Some(j.4);

        let start_time = Instant::now();
        let start = out_buf.seek(SeekFrom::Current(0)).unwrap();
        headers_location = match headers.as_mut().unwrap().write_to_buffer(&mut out_buf) {
            Some(x) => NonZeroU64::new(x),
            None => None,
        };
        let end = out_buf.seek(SeekFrom::Current(0)).unwrap();
        let end_time = Instant::now();
        log::debug!(
            "DEBUG: Wrote {} bytes of headers in {:#?}",
            end - start,
            end_time - start_time
        );

        let start_time = Instant::now();
        let start = out_buf.seek(SeekFrom::Current(0)).unwrap();
        ids_location = match ids.as_mut().expect("No ids!").write_to_buffer(&mut out_buf) {
            Some(x) => NonZeroU64::new(x),
            None => None,
        };
        let end = out_buf.seek(SeekFrom::Current(0)).unwrap();
        let end_time = Instant::now();
        log::debug!(
            "DEBUG: Wrote {} bytes of ids in {:#?}",
            end - start,
            end_time - start_time
        );

        let start_time = Instant::now();
        let start = out_buf.seek(SeekFrom::Current(0)).unwrap();
        masking_location = match masking.as_mut().unwrap().write_to_buffer(&mut out_buf) {
            Some(x) => NonZeroU64::new(x),
            None => None,
        };
        let end = out_buf.seek(SeekFrom::Current(0)).unwrap();
        let end_time = Instant::now();
        log::debug!(
            "DEBUG: Wrote {} bytes of masking in {:#?}",
            end - start,
            end_time - start_time
        );

        // TODO: Comes out about 20% smaller but about 2x as long in time...
        /*

        let start_time = Instant::now();
        let start = out_buf.seek(SeekFrom::Current(0)).unwrap();
        masking_location = match masking.as_mut().unwrap().write_to_buffer_zstd(&mut out_buf) {
            Some(x) => NonZeroU64::new(x),
            None => None,
        };
        let end = out_buf.seek(SeekFrom::Current(0)).unwrap();
        let end_time = Instant::now();
        log::debug!(
            "DEBUG: Wrote {} bytes of masking ZSTD in {:#?}",
            end - start,
            end_time - start_time
        ); */
    })
    .expect("Error");

    (
        ids_string,
        seq_locs.expect("Error"),
        block_index_pos.expect("Error"),
        headers_location,
        ids_location,
        masking_location,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_types::*;
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
