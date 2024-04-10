//! # Conversion Struct and Functions for FASTA/Q Files, with and without masking, scores, and indexing.
//!
//! Conversion functions for FASTA and FASTQ files. Multithreaded by default.

// Easy, high-performance conversion functions
use crossbeam::{queue::ArrayQueue, thread, utils::Backoff};
use needletail::parse_fastx_reader;
use xxhash_rust::xxh3::{self, xxh3_64};

use std::{
    io::{BufReader, Read, Seek, SeekFrom, Write},
    num::NonZeroU64,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
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
    pub fn convert<'convert, W, R>(self, in_buf: &mut R, mut out_fh: Box<W>) -> Box<W>
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

        let (seqlocs, headers_location, ids_location, masking_location, sequences_location, ids_to_locs) =
            self.process(in_buf, Arc::clone(&out_buffer));

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

            let index_handle = Some(s.spawn(|_| {
                for (id, loc) in ids_to_locs.into_iter() {
                    let id = xxh3_64(&id);
                    indexer.insert(id as u32, loc as u32);
                }
                indexer.flush_all();

                indexer
            }));

            let start_time = std::time::Instant::now();
            let start = out_buffer_thread.stream_position().unwrap();

            seqlocs_location = seqlocs.write_to_buffer(&mut *out_buffer_thread).unwrap();
            log::info!(
                "Writing SeqLocs to file: COMPLETE. {}",
                out_buffer_thread.stream_position().unwrap()
            );

            let end_time = std::time::Instant::now();
            log::info!("SeqLocs write time: {:?}", end_time - start_time);

            let end = out_buffer_thread.stream_position().unwrap();
            debug_size.push(("seqlocs".to_string(), (end - start) as usize));

            if self.index {
                log::info!("Joining index");
                let index = index_handle.unwrap().join().unwrap();
                log::info!(
                    "Writing index to file. {}",
                    out_buffer_thread.stream_position().unwrap()
                );

                let start = out_buffer_thread.stream_position().unwrap();

                let start_time = std::time::Instant::now();
                let mut index: FractalTreeDisk<u32, u32> = index.into();
                index.set_compression(CompressionConfig {
                    compression_type: CompressionType::ZSTD,
                    compression_level: -9,
                    compression_dict: None,
                });
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
        SeqLocsStoreBuilder,
        Option<NonZeroU64>,
        Option<NonZeroU64>,
        Option<NonZeroU64>,
        Option<NonZeroU64>,
        Vec<(std::sync::Arc<Vec<u8>>, usize)>, // todo: cow?
    )
    where
        W: WriteAndSeek + 'convert + Send + Sync + 'static,
        R: Read + Send + 'convert,
    {
        // TODO: Untested, been awhile... Only useful for very small blocks so hasn't been used lately...
        if let Some(_dict) = &self.dict {
            todo!();
            // sb_config = sb_config.with_compression_dict(dict.clone());
        }

        let threads = self.threads;

        let output_buffer = Arc::clone(&out_fh);

        // Start the output I/O...
        let mut output_worker = crate::io::worker::Worker::new(output_buffer).with_buffer_size(128);
        output_worker.start();
        let output_queue = output_worker.get_queue();

        // Start the compression workers
        let mut compression_workers = CompressionWorker::new()
            .with_buffer_size(128)
            .with_threads(threads as u16)
            .with_output_queue(Arc::clone(&output_queue));

        compression_workers.start();
        let compression_workers = Arc::new(compression_workers);

        // Some defaults
        let mut headers_location = None;
        let mut ids_location = None;
        let ids_string = Arc::new(RwLock::new(Vec::new()));
        let mut masking_location = None;
        let mut sequences_location = None;
        let ids_to_locs_o = Arc::new(RwLock::new(Vec::new()));

        // Sequence queue for the generator
        let seq_queue = crossbeam::channel::bounded(1024);
        let seq_queue_in = Arc::new(&seq_queue.0);
        let seq_queue_out = Arc::new(&seq_queue.1);

        // Pop to a new thread that pushes sequence into the sequence buffer...
        thread::scope(|s| {
            let fasta_thread = s.spawn(|_| {
                // let mut in_buf_reader = BufReader::new(in_buf);

                // TODO: Should be auto-detect and handle...
                // let fasta = Fasta::from_buffer(&mut in_buf_reader);
                let mut fastx = parse_fastx_reader(in_buf).expect("Unable to parse FASTA/Q file");

                let backoff = Backoff::new();

                while let Some(r) = fastx.next() {
                    let record = r.expect("Invalid FASTA record");

                    let mut d = Work::FastaPayload((record.id().to_vec(), record.seq().to_vec()));
                    while let Err(z) = seq_queue_in.try_send(d) {

                        match z {
                            crossbeam::channel::TrySendError::Full(x) => {
                                d = x;
                            }
                            crossbeam::channel::TrySendError::Disconnected(_) => {
                                panic!("Fastx Disconnected");
                            }
                        }

                        backoff.snooze();

                        if backoff.is_completed() {
                            std::thread::sleep(std::time::Duration::from_millis(60));
                            backoff.reset();
                        }
                    }
                }

                log::info!("FASTA reading complete...");
                while seq_queue_in.try_send(Work::Shutdown).is_err() {
                    backoff.snooze();
                    // todo handle better so it never gets stuck
                }
            });

            let fasta_thread_clone = fasta_thread.thread().clone();

            // TODO: Multithread this part -- maybe, now with compression logic into a threadpool maybe not...
            // Idea: split into queue's for headers, IDs, sequence, etc....
            // Thread that handles the heavy lifting of the incoming Seq structs
            // TODO: Set compression stuff here...
            // And batch this part...

            // 4 threads for this kind of stuff
            let compression_workers_thread = Arc::clone(&compression_workers);

            let headers_o = Arc::new(RwLock::new(
                StringBlockStoreBuilder::default()
                    .with_block_size(32 * 1024)
                    .with_compression_worker(Arc::clone(&compression_workers_thread)),
            ));
            let seqlocs_o = Arc::new(RwLock::new(SeqLocsStoreBuilder::default()));
            let ids_o = Arc::new(RwLock::new(
                StringBlockStoreBuilder::default()
                    .with_block_size(32 * 1024)
                    .with_compression_worker(Arc::clone(&compression_workers_thread)),
            ));
            // let mut ids_string = Vec::new();
            let masking_o = Arc::new(RwLock::new(
                MaskingStoreBuilder::default()
                    .with_compression_worker(Arc::clone(&compression_workers_thread))
                    .with_block_size(512 * 1024),
            ));
            let sequences_o = Arc::new(RwLock::new(
                SequenceBlockStoreBuilder::default()
                    .with_block_size(512 * 1024)
                    .with_compression_worker(Arc::clone(&compression_workers_thread)),
            ));

            let shutdown_o = Arc::new(AtomicBool::new(false));

            let mut handles = Vec::new();

            for _ in 0..2 {
                let masking = Arc::clone(&masking_o);
                let sequences = Arc::clone(&sequences_o);
                let seqlocs = Arc::clone(&seqlocs_o);
                let ids = Arc::clone(&ids_o);
                let headers = Arc::clone(&headers_o);
                let shutdown = Arc::clone(&shutdown_o);
                let seq_queue_out = Arc::clone(&seq_queue_out);
                let ids_string = Arc::clone(&ids_string);
                let fasta_thread_clone = fasta_thread_clone.clone();
                let ids_to_locs = Arc::clone(&ids_to_locs_o);

                let reader_handle = s.spawn(move |_| {
                    // For each Sequence in the fasta file, make it upper case (masking is stored separately)
                    // Add the sequence to the SequenceBlocks, get the SeqLocs and store them in Location
                    // struct And store that in seq_locs Vec...

                    let mut masking_times = Vec::new();
                    let mut seq_add_times = Vec::new();
                    let mut headers_time = Vec::new();
                    let mut ids_time = Vec::new();
                    let mut seqloc_time = Vec::new();
                    let mut seqlocs_time = Vec::new();

                    let backoff = Backoff::new();
                    // Turn the below into a closure

                    let mut add = |x: (Vec<u8>, Vec<u8>)| {
                        let mut seq = x.0;
                        let mut id = x.1.split(|x| *x == b' ');
                        // Split at first space
                        let seqid = id.next().unwrap();
                        let seqheader = id.next();

                        let start_time = std::time::Instant::now();
                        let mut masking_locs = Vec::new();
                        let mut masking = masking.write().unwrap();
                        if let Some(x) = masking.add_masking(&seq) {
                            masking_locs.extend(x);
                        }
                        drop(masking);
                        let end_time = std::time::Instant::now();
                        masking_times.push(end_time - start_time);

                        // Capitalize and add to sequences
                        let start_time = std::time::Instant::now();
                        let mut sequence_locs = Vec::new();
                        let mut sequences = sequences.write().unwrap();
                        seq.make_ascii_uppercase();
                        let loc = sequences.add(&mut seq);
                        sequence_locs.extend(loc);

                        let end_time = std::time::Instant::now();
                        seq_add_times.push(end_time - start_time);

                        let start_time = std::time::Instant::now();
                        let myid = std::sync::Arc::new(seqid.to_vec());
                        let mut ids = ids.write().unwrap();
                        let idloc = ids.add(&(*myid));
                        drop(ids);
                        let end_time = std::time::Instant::now();
                        ids_time.push(end_time - start_time);

                        let start_time = std::time::Instant::now();
                        let mut headers_loc = Vec::new();
                        if let Some(x) = seqheader {
                            let mut headers = headers.write().unwrap();
                            let x = headers.add(&x);
                            headers_loc.extend(x);
                        }
                        let end_time = std::time::Instant::now();
                        headers_time.push(end_time - start_time);

                        let mut seqloc = SeqLoc::new();

                        // TODO: This has become kinda gross
                        // Lots of vec's above, should be able to clean this up

                        let start_time = std::time::Instant::now();
                        seqloc.add_locs(&sequence_locs, &masking_locs, &[], &idloc, &headers_loc, &[], &[]);
                        let end_time = std::time::Instant::now();
                        seqloc_time.push(end_time - start_time);

                        let start_time = std::time::Instant::now();
                        let mut seqlocs = seqlocs.write().unwrap();
                        let mut ids_to_locs = ids_to_locs.write().unwrap();
                        let mut ids_string = ids_string.write().unwrap();

                        let loc = seqlocs.add_to_index(seqloc);
                        ids_string.push(Arc::clone(&myid));
                        ids_to_locs.push((myid, loc));
                        drop(ids_string);
                        drop(ids_to_locs);
                        drop(seqlocs);
                        let end_time = std::time::Instant::now();
                        seqlocs_time.push(end_time - start_time);
                    };

                    'outer: loop {
                        backoff.reset(); // Reset the backoff
                        match seq_queue_out.try_recv() {
                            Ok(Work::FastaPayload(seq)) => {
                                add(seq);
                            }

                            Ok(Work::FastqPayload(_)) => {
                                panic!("Received FASTQ payload in FASTA thread")
                            }
                            Ok(Work::Shutdown) => {
                                shutdown.store(true, Ordering::SeqCst);
                                break 'outer;
                            }
                            Err(x) => {
                                if let crossbeam::channel::TryRecvError::Disconnected = x {
                                    panic!("Disconnected");
                                }

                                if shutdown.load(Ordering::SeqCst) {
                                    break 'outer;
                                }

                                fasta_thread_clone.unpark();
                                backoff.snooze();
                                if backoff.is_completed() {
                                    log::debug!("FASTA queue empty");
                                    backoff.reset();
                                    std::thread::park();
                                }
                            }
                        }
                    }

                    // Get mean,
                    let mean = |x: &Vec<std::time::Duration>| {
                        let sum: std::time::Duration = x.iter().sum();
                        sum / x.len() as u32
                    };

                    log::info!("Masking times: {:?}", mean(&masking_times));
                    log::info!("Seq add times: {:?}", mean(&seq_add_times));
                    log::info!("Headers times: {:?}", mean(&headers_time));
                    log::info!("IDs times: {:?}", mean(&ids_time));
                    log::info!("SeqLoc times: {:?}", mean(&seqloc_time));
                    log::info!("SeqLocs times: {:?}", mean(&seqlocs_time));
                });

                handles.push(reader_handle);
            }

            let backoff = Backoff::new();
            while !shutdown_o.load(Ordering::SeqCst) {
                backoff.snooze();
                if backoff.is_completed() {
                    backoff.reset();
                    std::thread::sleep(std::time::Duration::from_millis(60));
                }
            }

            for handle in handles {
                handle.thread().unpark();
                handle.join().expect("Unable to join thread");
            }

            let mut headers = Arc::into_inner(headers_o).unwrap().into_inner().unwrap();

            let mut headers = match headers.write_block_locations() {
                Ok(x) => Some(headers),
                Err(x) => match x {
                    BlockStoreError::Empty => None,
                    _ => panic!("Error writing headers: {:?}", x),
                },
            };

            let mut ids = Arc::into_inner(ids_o).unwrap().into_inner().unwrap();

            let mut ids = match ids.write_block_locations() {
                Ok(x) => Some(ids),
                Err(x) => match x {
                    BlockStoreError::Empty => None,
                    _ => panic!("Error writing ids: {:?}", x),
                },
            };

            let mut masking = Arc::into_inner(masking_o).unwrap().into_inner().unwrap();
            let mut masking = match masking.write_block_locations() {
                Ok(x) => Some(masking),
                Err(x) => match x {
                    BlockStoreError::Empty => None,
                    _ => panic!("Error writing masking: {:?}", x),
                },
            };

            let mut sequences = Arc::into_inner(sequences_o).unwrap().into_inner().unwrap();
            let mut sequences = match sequences.write_block_locations() {
                Ok(x) => Some(sequences),
                Err(x) => match x {
                    BlockStoreError::Empty => None,
                    _ => panic!("Error writing sequences: {:?}", x),
                },
            };

            fasta_thread.join().expect("Unable to join thread");

            let seq_locs = Arc::into_inner(seqlocs_o).unwrap().into_inner().unwrap();

            let backoff = Backoff::new();
            while output_queue.len() > 0 {
                backoff.snooze();
            }

            output_worker.shutdown();

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

            let ids_to_locs = Arc::try_unwrap(ids_to_locs_o).expect("Unable to get ids_to_locs");
            let ids_to_locs = ids_to_locs.into_inner().expect("Unable to get ids_to_locs");

            (
                seq_locs,
                headers_location,
                ids_location,
                masking_location,
                sequences_location,
                ids_to_locs,
            )
        })
        .unwrap()
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
    FastaPayload((Vec<u8>, Vec<u8>)),
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
