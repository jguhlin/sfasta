//! # Conversion Struct and Functions for FASTA/Q Files, with and without masking, scores, and indexing.
//!
//! Conversion functions for FASTA and FASTQ files. Multithreaded by default.

// Easy, high-performance conversion functions
use crossbeam::{queue::ArrayQueue, thread, utils::Backoff};
use xxhash_rust::xxh3::{self, xxh3_64};

use std::{
    io::{BufReader, Read, Seek, SeekFrom, Write},
    num::NonZeroU64,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, RwLock,
    },
};

use crate::{datatypes::*, formats::*};
use libcompression::*;
use libfractaltree::{FractalTreeBuild, FractalTreeDisk};

/// Main conversion struct
///
/// Follows a builder style pattern, with default settings
/// ```rust
/// use libsfasta::conversion::Converter;
///
/// let mut converter = Converter::default()
///    .with_threads(4);
/// ```
pub struct Converter
{
    masking: bool,
    index: bool,
    threads: usize,
    pub block_size: usize,
    seqlocs_chunk_size: usize,
    quality_scores: bool,
    compression_type: CompressionType,
    compression_level: Option<i8>,
    dict: Option<Vec<u8>>,
}

/// Default settings for Converter
///
/// * threads: 4
/// * block_size: 4Mb
/// * seqlocs_chunk_size: 128k
/// * index: on
/// * masking: off
/// * quality scores: off
/// * compression_type: ZSTD
/// * compression_level: Default (3 for ZSTD)
impl Default for Converter
{
    fn default() -> Self
    {
        Converter {
            threads: 8,
            block_size: 4 * 1024 * 1024,    // 4Mb
            seqlocs_chunk_size: 256 * 1024, // 256k
            index: true,
            masking: false,
            quality_scores: false,
            compression_type: CompressionType::ZSTD,
            dict: None,
            compression_level: None,
        }
    }
}

impl Converter
{
    // Builder configuration functions...
    /// Specify a dictionary to use for compression. Untested.
    pub fn with_dict(mut self, dict: Vec<u8>) -> Self
    {
        self.dict = Some(dict);
        self
    }

    /// Disable dictionary
    pub fn without_dict(mut self) -> Self
    {
        self.dict = None;
        self
    }

    /// Enable masking
    pub fn with_masking(mut self) -> Self
    {
        self.masking = true;
        self
    }

    /// Enable seq index
    pub fn with_index(mut self) -> Self
    {
        self.index = true;
        self
    }

    /// Disable index
    pub fn without_index(mut self) -> Self
    {
        self.index = false;
        self
    }

    /// Enable quality scores
    pub fn with_scores(mut self) -> Self
    {
        self.quality_scores = true;
        self
    }

    /// Disable quality scores
    pub fn without_scores(mut self) -> Self
    {
        self.quality_scores = false;
        self
    }

    /// Set the number of threads to use
    pub fn with_threads(mut self, threads: usize) -> Self
    {
        assert!(
            threads < u16::MAX as usize,
            "Maximum number of supported threads is u16::MAX"
        );
        self.threads = threads;
        self
    }

    /// Set the block size for the sequence blocks
    pub fn with_block_size(mut self, block_size: usize) -> Self
    {
        assert!(
            block_size < u32::MAX as usize,
            "Block size must be less than u32::MAX (~4Gb)"
        );

        self.block_size = block_size;
        self
    }

    /// Set the chunk size for the sequence locations
    pub fn with_seqlocs_chunk_size(mut self, chunk_size: usize) -> Self
    {
        assert!(
            chunk_size < u32::MAX as usize,
            "Chunk size must be less than u32::MAX (~4Gb)"
        );

        self.seqlocs_chunk_size = chunk_size;

        self
    }

    /// Set the compression type
    pub fn with_compression_type(mut self, ct: CompressionType) -> Self
    {
        self.compression_type = ct;
        self
    }

    /// Set the compression level
    pub fn with_compression_level(mut self, level: i8) -> Self
    {
        self.compression_level = Some(level);
        self
    }

    /// Reset compression level to default
    pub fn with_default_compression_level(mut self) -> Self
    {
        self.compression_level = None;
        self
    }

    /// Write the headers for the SFASTA file, and return the location of th headers (so they can be updated at the end)
    fn write_headers<W>(&self, mut out_fh: &mut W, sfasta: &Sfasta) -> u64
    where
        W: Write + Seek,
    {
        // IMPORTANT: Headers must ALWAYS be fixed int encoding
        let bincode_config = crate::BINCODE_CONFIG.with_fixed_int_encoding();

        out_fh
            .write_all("sfasta".as_bytes())
            .expect("Unable to write 'sfasta' to output");

        // Write the directory, parameters, and metadata structs out...

        // Write the version
        bincode::encode_into_std_write(sfasta.version, &mut out_fh, bincode_config)
            .expect("Unable to write directory to file");

        // Write the directory
        let directory_location = out_fh.stream_position().expect("Unable to work with seek API");

        let dir: DirectoryOnDisk = sfasta.directory.clone().into();
        bincode::encode_into_std_write(dir, &mut out_fh, bincode_config).expect("Unable to write directory to file");

        // Write the parameters
        bincode::encode_into_std_write(&sfasta.parameters, &mut out_fh, bincode_config)
            .expect("Unable to write Parameters to file");

        // Write the metadata
        bincode::encode_into_std_write(&sfasta.metadata, &mut out_fh, bincode_config)
            .expect("Unable to write Metadata to file");

        // Return the directory location
        directory_location
    }

    /// Main conversion function for FASTA/Q files
    pub fn convert<'convert, W, R>(self, mut in_buf: &mut R, mut out_fh: Box<W>) -> Box<W>
    where
        W: WriteAndSeek + 'static + Send + Sync,
        R: Read + Send + 'convert,
    {
        // Keep track of how long the conversion takes
        let conversion_start_time = std::time::Instant::now();

        // Track how much space each individual element takes up
        let mut debug_size: Vec<(String, usize)> = Vec::new();

        assert!(self.block_size < u32::MAX as usize);

        let mut sfasta = Sfasta::default().block_size(self.block_size as u32);

        // Store masks as series of 0s and 1s... Vec<bool>
        // Compression seems to take care of the size. bitvec! and vec! seem to have similar
        // performance and on-disk storage requirements
        if self.masking {
            sfasta = sfasta.with_masking();
        }

        sfasta.parameters.compression_type = self.compression_type;
        sfasta.parameters.compression_dict = self.dict.clone();
        sfasta.parameters.seqlocs_chunk_size = self.seqlocs_chunk_size as u32;
        sfasta.parameters.block_size = self.block_size as u32;

        // Set dummy values for the directory
        sfasta.directory.dummy();

        // Store the location of the directory so we can update it later...
        // It's right after the SFASTA and version identifier...
        let directory_location = self.write_headers(&mut out_fh, &sfasta);

        // Put everything in Arcs and Mutexes to allow for multithreading
        // let in_buf = Arc::new(Mutex::new(in_buf));
        // let out_buf = Arc::new(Mutex::new(out_buf));

        log::info!("Writing sequences start... {}", out_fh.stream_position().unwrap());

        // Calls the big function write_fasta_sequence to process both sequences and masking, and write them into a
        // file.... Function returns:
        // Vec<(String, Location)>
        // block_index_pos

        let out_buffer = Arc::new(Mutex::new(out_fh));

        let (ids, mut seqlocs, headers_location, ids_location, masking_location, sequences_location, ids_to_locs) =
            self.process(&mut in_buf, Arc::clone(&out_buffer));

        // TODO: Here is where we would write out the Seqinfo stream (if it's decided to do it)

        // The index points to the location of the Location structs.
        // Location blocks are chunked into SEQLOCS_CHUNK_SIZE
        // So index.get(id) --> integer position of the location struct
        // Which will be in integer position / SEQLOCS_CHUNK_SIZE chunk offset
        // This will then point to the different location blocks where the sequence is...
        //
        // TODO: Optional index can probably be handled better...
        let mut fractaltree_pos = 0;

        let mut seqlocs_location = 0;

        let mut out_buffer_thread = out_buffer.lock().unwrap();

        // Build the index in another thread...
        thread::scope(|s| {
            let mut indexer = libfractaltree::FractalTreeBuild::new(128, 256);

            // Use the main thread to write the sequence locations...
            log::info!(
                "Writing SeqLocs to file. {}",
                out_buffer_thread.stream_position().unwrap()
            );

            let start_time = std::time::Instant::now();
            let start = out_buffer_thread.stream_position().unwrap();

            let index_handle = Some(s.spawn(|_| {
                let backoff = Backoff::new();
                for (id, loc) in ids_to_locs.into_iter() {
                    let id = xxh3_64(id.as_bytes());
                    while loc.load(Ordering::Relaxed) == 0 {
                        backoff.snooze();
                        if backoff.is_completed() {
                            std::thread::yield_now();
                            backoff.reset();
                        }
                    }
                    indexer.insert(id as u32, loc.load(Ordering::Relaxed) as u32);
                }
                indexer.flush_all();

                indexer
            }));

            seqlocs_location = seqlocs.write_to_buffer(&mut *out_buffer_thread);
            log::info!(
                "Writing SeqLocs to file: COMPLETE. {}",
                out_buffer_thread.stream_position().unwrap()
            );

            let end_time = std::time::Instant::now();
            log::info!("SeqLocs write time: {:?}", end_time - start_time);

            let end = out_buffer_thread.stream_position().unwrap();
            debug_size.push(("seqlocs".to_string(), (end - start) as usize));

            if self.index {
                let index = index_handle.unwrap().join().unwrap();
                log::info!(
                    "Writing index to file. {}",
                    out_buffer_thread.stream_position().unwrap()
                );

                let start = out_buffer_thread.stream_position().unwrap();

                let start_time = std::time::Instant::now();
                let mut index: FractalTreeDisk = index.into();
                println!("Index: {:?}", index.len());

                fractaltree_pos = index
                    .write_to_buffer(&mut *out_buffer_thread)
                    .expect("Unable to write index to file");

                let end_time = std::time::Instant::now();
                log::info!("Index write time: {:?}", end_time - start_time);

                let end = out_buffer_thread.stream_position().unwrap();
                debug_size.push(("index".to_string(), (end - start) as usize));
            }
        })
        .expect("Error");

        drop(out_buffer_thread);

        sfasta.directory.seqlocs_loc = NonZeroU64::new(seqlocs_location);
        sfasta.directory.index_loc = NonZeroU64::new(fractaltree_pos);
        sfasta.directory.headers_loc = headers_location;
        sfasta.directory.ids_loc = ids_location;
        sfasta.directory.masking_loc = masking_location;
        sfasta.directory.sequences_loc = sequences_location;

        // Go to the beginning, and write the location of the index

        // sfasta.directory.block_index_loc = NonZeroU64::new(block_index_pos);

        let mut out_buffer_lock = out_buffer.lock().unwrap();
        out_buffer_lock
            .seek(SeekFrom::Start(directory_location))
            .expect("Unable to rewind to start of the file");

        // Here we re-write the directory information at the start of the file, allowing for
        // easy jumps to important areas while keeping everything in a single file

        let start = out_buffer_lock.stream_position().unwrap();

        let start_time = std::time::Instant::now();

        let bincode_config_fixed = crate::BINCODE_CONFIG.with_fixed_int_encoding();

        let dir: DirectoryOnDisk = sfasta.directory.clone().into();
        bincode::encode_into_std_write(dir, &mut *out_buffer_lock, bincode_config_fixed)
            .expect("Unable to write directory to file");

        let end = out_buffer_lock.stream_position().unwrap();
        debug_size.push(("directory".to_string(), (end - start) as usize));

        let end_time = std::time::Instant::now();

        log::info!("Directory write time: {:?}", end_time - start_time);

        log::info!("DEBUG: {:?}", debug_size);

        out_buffer_lock.flush().expect("Unable to flush output file");

        let conversion_end_time = std::time::Instant::now();
        log::info!("Conversion time: {:?}", conversion_end_time - conversion_start_time);

        drop(out_buffer_lock);

        // Pull out of Arc and Mutex
        match Arc::try_unwrap(out_buffer) {
            Ok(x) => x.into_inner().unwrap(),
            Err(_) => panic!("Unable to unwrap out_buffer"),
        }
    }

    /// Process buffer that outputs Seq objects
    pub fn process<'convert, W, R>(
        &self,
        in_buf: &mut R,
        out_fh: Arc<Mutex<Box<W>>>,
    ) -> (
        Vec<std::sync::Arc<String>>,
        SeqLocsStoreBuilder,
        Option<NonZeroU64>,
        Option<NonZeroU64>,
        Option<NonZeroU64>,
        Option<NonZeroU64>,
        Vec<(std::sync::Arc<String>, Arc<AtomicU64>)>,
    )
    where
        W: WriteAndSeek + 'convert + Send + Sync + 'static,
        R: Read + Send + 'convert,
    {
        // TODO: Untested, been awhile... Only useful for very small blocks so hasn't been used lately...
        if let Some(dict) = &self.dict {
            todo!();
            // sb_config = sb_config.with_compression_dict(dict.clone());
        }

        let threads = self.threads;

        let output_buffer = Arc::clone(&out_fh);

        // Start the output I/O...
        let mut output_worker = crate::io::worker::Worker::new(output_buffer).with_buffer_size(1024);
        output_worker.start();
        let output_queue = output_worker.get_queue();

        // Start the compression workers
        let mut compression_workers = CompressionWorker::new()
            .with_buffer_size(8192)
            .with_threads(threads as u16)
            .with_output_queue(Arc::clone(&output_queue));

        compression_workers.start();
        let compression_workers = Arc::new(compression_workers);

        // Some defaults
        let mut seq_locs: Option<SeqLocsStoreBuilder> = None;
        let mut headers = None;
        let mut headers_location = None;
        let mut ids = None;
        let mut ids_location = None;
        let mut ids_string = Vec::new();
        let mut masking = None;
        let mut masking_location = None;
        let mut sequences = None;
        let mut sequences_location = None;
        let ids_to_locs = Arc::new(RwLock::new(Vec::new()));

        // Sequence queue for the generator
        let seq_queue: Arc<ArrayQueue<Work>> = std::sync::Arc::new(ArrayQueue::new(8192 * 2));
        let seq_queue_in = Arc::clone(&seq_queue);
        let seq_queue_out = Arc::clone(&seq_queue);

        // Pop to a new thread that pushes sequence into the sequence buffer...
        thread::scope(|s| {
            let ids_to_locs = Arc::clone(&ids_to_locs);
            let fasta_thread = s.spawn(|_| {
                let mut in_buf_reader = BufReader::new(in_buf);

                // TODO: Should be auto-detect and handle...
                let fasta = Fasta::from_buffer(&mut in_buf_reader);

                let backoff = Backoff::new();

                for x in fasta {
                    backoff.reset();
                    if x.is_err() {
                        while seq_queue_in.push(Work::Shutdown).is_err() {
                            backoff.snooze();
                        }

                        log::error!("Error reading FASTA file: {:?}", x);
                        panic!("Error reading FASTA file: {:?}", x);
                    }

                    let mut d = Work::FastaPayload(x.unwrap());
                    while let Err(z) = seq_queue_in.push(d) {
                        d = z;
                        backoff.snooze();

                        if backoff.is_completed() {
                            std::thread::park();
                            backoff.reset();
                        }
                    }
                }

                log::info!("FASTA reading complete...");
                while seq_queue_in.push(Work::Shutdown).is_err() {
                    backoff.snooze();
                }
            });

            let fasta_thread_clone = fasta_thread.thread().clone();

            // TODO: Multithread this part -- maybe, now with compression logic into a threadpool maybe not...
            // Idea: split into queue's for headers, IDs, sequence, etc....
            // Thread that handles the heavy lifting of the incoming Seq structs
            // TODO: Set compression stuff here...
            let compression_workers_thread = Arc::clone(&compression_workers);
            let reader_handle = s.spawn(move |_| {
                let mut ids_to_locs = ids_to_locs.write().unwrap();

                let mut headers = StringBlockStoreBuilder::default()
                    .with_block_size(512 * 1024)
                    .with_compression_worker(Arc::clone(&compression_workers_thread));
                let mut seqlocs = SeqLocsStoreBuilder::default();
                let mut ids = StringBlockStoreBuilder::default()
                    .with_block_size(512 * 1024)
                    .with_compression_worker(Arc::clone(&compression_workers_thread));
                let mut ids_string = Vec::new();
                let mut masking =
                    MaskingStoreBuilder::default().with_compression_worker(Arc::clone(&compression_workers_thread));
                let mut sequences = SequenceBlockStoreBuilder::default()
                    .with_block_size(512 * 1024)
                    .with_compression_worker(Arc::clone(&compression_workers_thread));

                // For each Sequence in the fasta file, make it upper case (masking is stored separately)
                // Add the sequence to the SequenceBlocks, get the SeqLocs and store them in Location
                // struct And store that in seq_locs Vec...

                let backoff = Backoff::new();
                'outer: loop {
                    backoff.reset(); // Reset the backoff
                    fasta_thread_clone.unpark(); // Unpark the FASTA thread so it can continue to read
                    match seq_queue_out.pop() {
                        Some(Work::FastaPayload(seq)) => {
                            let (seqid, seqheader, seq, _) = seq.into_parts();

                            let mut seqloc = SeqLoc::new();

                            let masked = masking.add_masking(&seq.as_ref().unwrap()[..]);
                            if let Some(x) = masked {
                                seqloc.add_masking_locs(x);
                            }

                            // Capitalize sequence
                            if let Some(mut x) = seq {
                                x.make_ascii_uppercase();
                                let loc = sequences.add(&mut x[..]);
                                seqloc.add_sequence_locs(loc);
                            }

                            let myid = std::sync::Arc::new(seqid.unwrap());
                            ids_string.push(std::sync::Arc::clone(&myid));
                            let idloc = ids.add(&(*myid));

                            if let Some(x) = seqheader {
                                let x = headers.add(&x);
                                seqloc.add_header_locs(x);
                            }

                            seqloc.add_id_locs(idloc);

                            let loc = seqlocs.add_to_index(seqloc);
                            ids_to_locs.push((myid, loc));
                        }
                        Some(Work::FastqPayload(_)) => {
                            panic!("Received FASTQ payload in FASTA thread")
                        }
                        Some(Work::Shutdown) => break 'outer,
                        None => {
                            fasta_thread_clone.unpark();
                            backoff.snooze();
                            if backoff.is_completed() {
                                backoff.reset();
                                std::thread::park();
                            }
                        }
                    }
                }

                let headers = match headers.write_block_locations() {
                    Ok(x) => Some(headers),
                    Err(x) => match x {
                        BlockStoreError::Empty => None,
                        _ => panic!("Error writing headers: {:?}", x),
                    },
                };

                let ids = match ids.write_block_locations() {
                    Ok(x) => Some(ids),
                    Err(x) => match x {
                        BlockStoreError::Empty => None,
                        _ => panic!("Error writing ids: {:?}", x),
                    },
                };

                let masking = match masking.write_block_locations() {
                    Ok(x) => Some(masking),
                    Err(x) => match x {
                        BlockStoreError::Empty => None,
                        _ => panic!("Error writing masking: {:?}", x),
                    },
                };

                let sequences = match sequences.write_block_locations() {
                    Ok(x) => Some(sequences),
                    Err(x) => match x {
                        BlockStoreError::Empty => None,
                        _ => panic!("Error writing sequences: {:?}", x),
                    },
                };

                (seqlocs, headers, ids, ids_string, masking, sequences)
            });

            // Join the FASTA thread (ot block until it can be joined)
            fasta_thread.join().unwrap();

            reader_handle.thread().unpark();
            let j = reader_handle.join().expect("Unable to join thread");

            let backoff = Backoff::new();
            while output_queue.len() > 0 {
                backoff.snooze();
            }

            output_worker.shutdown();

            seq_locs = Some(j.0);
            headers = j.1;
            ids = j.2;
            ids_string = j.3;
            masking = j.4;
            sequences = j.5;

            let mut out_buffer = out_fh.lock().unwrap();

            // Write the headers with the complete information now...

            if headers.is_some() {
                let x = headers.as_mut().unwrap();
                headers_location = NonZeroU64::new(out_buffer.stream_position().unwrap());
                x.write_header(headers_location.unwrap().get(), &mut *out_buffer);
            } else {
                headers_location = None;
            }

            if ids.is_some() {
                let x = ids.as_mut().unwrap();
                ids_location = NonZeroU64::new(out_buffer.stream_position().unwrap());
                x.write_header(ids_location.unwrap().get(), &mut *out_buffer);
            } else {
                ids_location = None;
            }

            if masking.is_some() {
                let x = masking.as_mut().unwrap();
                masking_location = NonZeroU64::new(out_buffer.stream_position().unwrap());
                x.write_header(masking_location.unwrap().get(), &mut *out_buffer);
            } else {
                masking_location = None;
            }

            if sequences.is_some() {
                let x = sequences.as_mut().unwrap();
                sequences_location = NonZeroU64::new(out_buffer.stream_position().unwrap());
                x.write_header(sequences_location.unwrap().get(), &mut *out_buffer);
            } else {
                sequences_location = None;
            }

            out_buffer.flush().expect("Unable to flush output buffer");
        })
        .expect("Error");

        let ids_to_locs = Arc::try_unwrap(ids_to_locs).expect("Unable to get ids_to_locs");
        let ids_to_locs = ids_to_locs.into_inner().expect("Unable to get ids_to_locs");

        (
            ids_string,
            seq_locs.expect("Error"),
            headers_location,
            ids_location,
            masking_location,
            sequences_location,
            ids_to_locs,
        )
    }
}

// TODO: Add support for metadata here...
// TODO: Will likely need to be the same builder style
// TODO: Will need to generalize this function so it works with FASTA & FASTQ & Masking

/// Input filehandle goes in, output goes out.
/// Function returns:
/// Vec<(String, Location)>
/// block_index_pos
/// in_buffer
/// out_buffer

#[derive(Debug)]
enum Work
{
    FastaPayload(crate::datatypes::Sequence),
    FastqPayload(crate::datatypes::Sequence), // TODO
    Shutdown,
}

#[cfg(test)]
mod tests
{
    use super::*;
    use std::{fs::File, io::Cursor};

    fn init()
    {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    pub fn test_create_sfasta()
    {
        init();

        let bincode_config = crate::BINCODE_CONFIG.with_fixed_int_encoding();

        let out_buf = Box::new(Cursor::new(Vec::new()));

        println!("test_data/test_convert.fasta");
        let mut in_buf =
            BufReader::new(File::open("test_data/test_convert.fasta").expect("Unable to open testing file"));

        let converter = Converter::default().with_threads(6).with_block_size(8192);

        let mut out_buf = converter.convert(&mut in_buf, out_buf);

        if let Err(x) = out_buf.seek(SeekFrom::Start(0)) {
            panic!("Unable to seek to start of file, {:#?}", x)
        };

        let mut sfasta_marker: [u8; 6] = [0; 6];
        out_buf
            .read_exact(&mut sfasta_marker)
            .expect("Unable to read SFASTA Marker");
        assert!(sfasta_marker == "sfasta".as_bytes());

        let _version: u64 = bincode::decode_from_std_read(&mut out_buf, bincode_config).unwrap();

        let directory: DirectoryOnDisk = bincode::decode_from_std_read(&mut out_buf, bincode_config).unwrap();

        let _dir = directory;

        let _parameters: Parameters = bincode::decode_from_std_read(&mut out_buf, bincode_config).unwrap();

        let _metadata: Metadata = bincode::decode_from_std_read(&mut out_buf, bincode_config).unwrap();

        // TODO: Add more tests
    }
}
