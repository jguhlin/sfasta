//! # Conversion Struct and Functions for FASTA/Q Files, with and without masking, scores, and indexing.
//!
//! Conversion functions for FASTA and FASTQ files. Multithreaded by default.

// Easy, high-performance conversion functions
use crossbeam::queue::ArrayQueue;
use crossbeam::thread;
use crossbeam::utils::Backoff;

use std::fs::{metadata, File};
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::num::NonZeroU64;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use crate::compression_stream_buffer::{CompressionStreamBuffer, CompressionStreamBufferConfig};
use crate::datatypes::*;
use crate::dual_level_index::*;
use crate::formats::*;
use crate::utils::*;
use crate::CompressionType;

/// Main conversion struct
///
/// Follows a builder style pattern, with default settings
/// ```rust
/// use libsfasta::conversion::Converter;
///
/// let mut converter = Converter::default()
///    .with_threads(4);
/// ```
pub struct Converter {
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
impl Default for Converter {
    fn default() -> Self {
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

impl Converter {
    // Builder configuration functions...
    /// Specify a dictionary to use for compression. Untested.
    pub fn with_dict(mut self, dict: Vec<u8>) -> Self {
        self.dict = Some(dict);
        self
    }

    /// Disable dictionary
    pub fn without_dict(mut self) -> Self {
        self.dict = None;
        self
    }

    /// Enable masking
    pub fn with_masking(mut self) -> Self {
        self.masking = true;
        self
    }

    /// Enable seq index
    pub fn with_index(mut self) -> Self {
        self.index = true;
        self
    }

    /// Disable index
    pub fn without_index(mut self) -> Self {
        self.index = false;
        self
    }

    /// Enable quality scores
    pub fn with_scores(mut self) -> Self {
        self.quality_scores = true;
        self
    }

    /// Disable quality scores
    pub fn without_scores(mut self) -> Self {
        self.quality_scores = false;
        self
    }

    /// Set the number of threads to use
    pub fn with_threads(mut self, threads: usize) -> Self {
        assert!(
            threads < u16::MAX as usize,
            "Maximum number of supported threads is u16::MAX"
        );
        self.threads = threads;
        self
    }

    /// Set the block size for the sequence blocks
    pub fn with_block_size(mut self, block_size: usize) -> Self {
        assert!(
            block_size < u32::MAX as usize,
            "Block size must be less than u32::MAX (~4Gb)"
        );

        self.block_size = block_size;
        self
    }

    /// Set the chunk size for the sequence locations
    pub fn with_seqlocs_chunk_size(mut self, chunk_size: usize) -> Self {
        assert!(
            chunk_size < u32::MAX as usize,
            "Chunk size must be less than u32::MAX (~4Gb)"
        );

        self.seqlocs_chunk_size = chunk_size;

        self
    }

    /// Set the compression type
    pub fn with_compression_type(mut self, ct: CompressionType) -> Self {
        self.compression_type = ct;
        self
    }

    /// Set the compression level
    pub fn with_compression_level(mut self, level: i8) -> Self {
        self.compression_level = Some(level);
        self
    }

    /// Reset compression level to default
    pub fn with_default_compression_level(mut self) -> Self {
        self.compression_level = None;
        self
    }

    /// Write the headers for the SFASTA file, and return the location of th headers (so they can be updated at the end)
    fn write_headers<W>(&self, mut out_fh: &mut Box<W>, sfasta: &Sfasta) -> u64
    where
        W: Write + Seek,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        out_fh
            .write_all("sfasta".as_bytes())
            .expect("Unable to write 'sfasta' to output");

        // Write the directory, parameters, and metadata structs out...

        // Write the version
        bincode::encode_into_std_write(sfasta.version, &mut out_fh, bincode_config)
            .expect("Unable to write directory to file");

        // Write the directory
        let directory_location = out_fh
            .stream_position()
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

    /// Main conversion function for FASTA files.
    // TODO: Switch R and W into borrows
    pub fn convert_fasta<'convert, W, R>(self, mut in_buf: R, mut out_fh: &mut Box<W>)
    where
        W: WriteAndSeek + 'static,
        R: Read + Send + 'convert,
    {
        let fn_start_time = std::time::Instant::now();

        let mut debug_size: Vec<(String, usize)> = Vec::new();

        // Nearly all of this needs to be fixed int encoding
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        assert!(self.block_size < u32::MAX as usize);

        let mut sfasta = Sfasta::default().block_size(self.block_size as u32);

        // Store masks as series of 0s and 1s... Vec<bool>
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
        let directory_location = self.write_headers(out_fh, &sfasta);

        // Put everything in Arcs and Mutexes to allow for multithreading
        // let in_buf = Arc::new(Mutex::new(in_buf));
        // let out_buf = Arc::new(Mutex::new(out_buf));

        log::info!(
            "Writing sequences start... {}",
            out_fh.stream_position().unwrap()
        );

        let start = out_fh.stream_position().unwrap();

        // Write sequences

        let start_time = std::time::Instant::now();

        // Calls the big function write_fasta_sequence to process both sequences and masking, and write them into a file....
        // Function returns:
        // Vec<(String, Location)>
        // block_index_pos
        let (ids, mut seqlocs, block_index_pos, headers_location, ids_location, masking_location) =
            self.write_sequence(&mut in_buf, out_fh, &mut debug_size);

        let end_time = std::time::Instant::now();

        log::info!(
            "Write Fasta Sequence write time: {:?}",
            end_time - start_time
        );

        let end = out_fh.stream_position().unwrap();

        debug_size.push(("sequences".to_string(), (end - start) as usize));

        log::info!(
            "Writing sequences finished... {}",
            out_fh.stream_position().unwrap()
        );

        // TODO: Here is where we would write out the Seqinfo stream (if it's decided to do it)

        // TODO: Support for Index32 (and even smaller! What if only 1 or 2 sequences?)
        let mut indexer =
            crate::dual_level_index::DualIndexBuilder::with_capacity(seqlocs.index_len());

        // The index points to the location of the Location structs.
        // Location blocks are chunked into SEQLOCS_CHUNK_SIZE
        // So index.get(id) --> integer position of the location struct
        // Which will be in integer position / SEQLOCS_CHUNK_SIZE chunk offset
        // This will then point to the different location blocks where the sequence is...
        //
        // TODO: Optional index can probably be handled better...
        let mut dual_index_pos = 0;

        let mut seqlocs_location = 0;

        // Build the index in another thread...
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
            log::info!(
                "Writing SeqLocs to file. {}",
                out_fh.stream_position().unwrap()
            );

            let start = out_fh.stream_position().unwrap();

            let start_time = std::time::Instant::now();

            seqlocs_location = seqlocs.write_to_buffer(&mut out_fh);
            log::info!(
                "Writing SeqLocs to file: COMPLETE. {}",
                out_fh.stream_position().unwrap()
            );

            let end_time = std::time::Instant::now();
            log::info!("SeqLocs write time: {:?}", end_time - start_time);

            let end = out_fh.stream_position().unwrap();
            debug_size.push(("seqlocs".to_string(), (end - start) as usize));

            if self.index {
                dual_index_pos = out_fh
                    .stream_position()
                    .expect("Unable to work with seek API");

                let mut index = index_handle.unwrap().join().unwrap();
                log::info!(
                    "Writing index to file. {}",
                    out_fh.stream_position().unwrap()
                );

                let start = out_fh.stream_position().unwrap();

                let start_time = std::time::Instant::now();
                index.write_to_buffer(&mut out_fh);
                let end_time = std::time::Instant::now();
                log::info!("Index write time: {:?}", end_time - start_time);

                let end = out_fh.stream_position().unwrap();
                debug_size.push(("index".to_string(), (end - start) as usize));

                log::info!(
                    "Writing index to file: COMPLETE. {}",
                    out_fh.stream_position().unwrap()
                );
            }
        })
        .expect("Error");

        sfasta.directory.seqlocs_loc = NonZeroU64::new(seqlocs_location);
        sfasta.directory.index_loc = NonZeroU64::new(dual_index_pos);
        sfasta.directory.headers_loc = headers_location;
        sfasta.directory.ids_loc = ids_location;
        sfasta.directory.masking_loc = masking_location;

        // Go to the beginning, and write the location of the index

        sfasta.directory.block_index_loc = NonZeroU64::new(block_index_pos);

        out_fh
            .seek(SeekFrom::Start(directory_location))
            .expect("Unable to rewind to start of the file");

        // Here we re-write the directory information at the start of the file, allowing for
        // easy jumps to important areas while keeping everything in a single file

        let start = out_fh.stream_position().unwrap();

        let start_time = std::time::Instant::now();

        let dir: DirectoryOnDisk = sfasta.directory.clone().into();
        bincode::encode_into_std_write(dir, &mut out_fh, bincode_config)
            .expect("Unable to write directory to file");

        let end = out_fh.stream_position().unwrap();
        debug_size.push(("directory".to_string(), (end - start) as usize));

        let end_time = std::time::Instant::now();

        log::info!("Directory write time: {:?}", end_time - start_time);

        log::info!("DEBUG: {:?}", debug_size);

        out_fh.flush().expect("Unable to flush output file");

        let fn_end_time = std::time::Instant::now();
        log::info!("Conversion time: {:?}", fn_end_time - fn_start_time);
    }

    /// Unified function for writing FASTA and FASTQ files
    // TODO In development
    pub fn write_sequence<'convert, W, R>(
        &self,
        in_buf: &mut R,
        mut out_fh: &mut Box<W>,
        debug_size: &mut Vec<(String, usize)>,
    ) -> (
        Vec<std::sync::Arc<String>>,
        SeqLocs,
        u64,
        Option<NonZeroU64>,
        Option<NonZeroU64>,
        Option<NonZeroU64>,
    )
    where
        W: WriteAndSeek + 'convert,
        R: Read + Send + 'convert,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        // Creates the sequence block compressor configuration
        let mut sb_config = CompressionStreamBufferConfig::default()
            .with_block_size(self.block_size as u32)
            .with_compression_type(self.compression_type);

        if self.compression_level.is_some() {
            sb_config = sb_config.with_compression_level(self.compression_level.unwrap());
        }

        // TODO: Untested, been awhile... Only useful for very small blocks so hasn't been used lately...
        if let Some(dict) = &self.dict {
            sb_config = sb_config.with_compression_dict(dict.clone());
        }

        let mut sb = CompressionStreamBuffer::from_config(sb_config);

        // Multithreading support
        let oq = sb.get_output_queue(); // Get a handle for the output queue from the sequence compressor
        let shutdown = sb.get_shutdown_flag(); // Get a handle to the shutdown flag...

        // Some defaults
        let mut seq_locs: Option<SeqLocs> = None;
        let mut block_index_pos = None;
        let mut headers = None;
        let mut headers_location = None;
        let mut ids = None;
        let mut ids_location = None;
        let mut ids_string = Vec::new();
        let mut masking = None;
        let mut masking_location = None;

        // Sequence queue for the generator
        let seq_queue: Arc<ArrayQueue<Work>> = std::sync::Arc::new(ArrayQueue::new(8192 * 2)); // TODO: Find best value here?
        let seq_queue_in = Arc::clone(&seq_queue);
        let seq_queue_out = Arc::clone(&seq_queue);

        // Pop to a new thread that pushes sequence into the sequence buffer...
        thread::scope(|s| {
            let fasta_thread = s.spawn(|_| {
                let mut fasta_thread_spins: usize = 0; // Debugging
                                                       // Convert reader into buffered reader then into the Fasta struct (and iterator)
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
                        fasta_thread_spins = fasta_thread_spins.saturating_add(1);

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

                log::info!("fasta_thread_spins: {}", fasta_thread_spins);
            });

            let fasta_thread_clone = fasta_thread.thread().clone();

            // TODO: Multithread this part
            // Idea: split into queue's for headers, IDs, sequence, etc....
            // Thread that handles the heavy lifting of the incoming Seq structs
            let reader_handle = s.spawn(move |_| {
                sb.initialize();

                let mut headers = StringBlockStore::default().with_block_size(512 * 1024);
                let mut seqlocs = SeqLocs::default();
                let mut ids = StringBlockStore::default().with_block_size(512 * 1024);
                let mut ids_string = Vec::new();
                let mut masking = Masking::default();

                // For each Sequence in the fasta file, make it upper case (masking is stored separately)
                // Add the sequence to the SequenceBlocks, get the SeqLocs and store them in Location struct
                // And store that in seq_locs Vec...

                let backoff = Backoff::new();

                let mut masking_time = std::time::Duration::new(0, 0);
                let mut adding_time = std::time::Duration::new(0, 0);
                let mut seq_loc_time = std::time::Duration::new(0, 0);
                let mut ids_add_time = std::time::Duration::new(0, 0);
                let mut seqlocs_add_time = std::time::Duration::new(0, 0);
                let mut headers_add_time = std::time::Duration::new(0, 0);

                let mut fasta_queue_spins: usize = 0;
                loop {
                    backoff.reset(); // Reset the backoff
                    fasta_thread_clone.unpark(); // Unpark the FASTA thread so it can continue to read
                    match seq_queue_out.pop() {
                        Some(Work::FastaPayload(seq)) => {
                            let (seqid, seqheader, seq, _) = seq.into_parts();

                            let mut location = SeqLoc::new();

                            let now = std::time::Instant::now();
                            let masked = masking.add_masking(&seq.as_ref().unwrap()[..]);
                            if let Some(x) = masked {
                                let x = seqlocs.add_locs(&x);
                                location.masking = Some(x);
                            }
                            masking_time += now.elapsed();

                            let now = std::time::Instant::now();
                            let loc = sb.add_sequence(&mut seq.unwrap()[..]).unwrap(); // Destructive, capitalizes everything...
                            adding_time += now.elapsed();

                            let now = std::time::Instant::now();
                            let myid = std::sync::Arc::new(seqid.unwrap());
                            ids_string.push(std::sync::Arc::clone(&myid));
                            let idloc = ids.add(&(*myid));
                            ids_add_time += now.elapsed();

                            let now = std::time::Instant::now();
                            if let Some(x) = seqheader {
                                let x = seqlocs.add_locs(&headers.add(&x));
                                location.headers = Some((x.0, x.1 as u8));
                            }
                            headers_add_time += now.elapsed();

                            let now = std::time::Instant::now();
                            let x = seqlocs.add_locs(&idloc);
                            location.ids = Some((x.0, x.1 as u8));
                            seqlocs_add_time += now.elapsed();

                            let now = std::time::Instant::now();
                            location.sequence = Some(seqlocs.add_locs(&loc));
                            seqlocs.add_to_index(location);
                            seq_loc_time += now.elapsed();
                        }
                        Some(Work::FastqPayload(_)) => {
                            panic!("Received FASTQ payload in FASTA thread")
                        }
                        Some(Work::Shutdown) => break,
                        None => {
                            fasta_thread_clone.unpark();
                            fasta_queue_spins = fasta_queue_spins.saturating_add(1);
                            backoff.snooze();
                            if backoff.is_completed() {
                                backoff.reset();
                                std::thread::park();
                            }
                        }
                    }
                }

                log::info!("Masking time: {}ms", masking_time.as_millis());
                log::info!("Adding time: {}ms", adding_time.as_millis());
                log::info!("SeqLoc time: {}ms", seq_loc_time.as_millis());
                log::info!("IDs add time: {}ms", ids_add_time.as_millis());
                log::info!("SeqLocs add time: {}ms", seqlocs_add_time.as_millis());
                log::info!("Headers add time: {}ms", headers_add_time.as_millis());
                log::info!("Finalizing SequenceBuffer");
                // Finalize pushes the last block, which is likely smaller than the complete block size
                match sb.finalize() {
                    Ok(()) => (),
                    Err(x) => panic!("Unable to finalize sequence buffer, {:#?}", x),
                };

                log::info!("fasta_queue_spins: {}", fasta_queue_spins);

                // Return seq_locs to the main thread
                (seqlocs, headers, ids, ids_string, masking)
            });

            // Store the location of the Sequence Blocks...
            // Stored as Vec<(u32, u64)> because multithreading means it does not have to be in order
            let mut block_locs = Vec::with_capacity(1024);
            let mut pos = out_fh
                .stream_position()
                .expect("Unable to work with seek API");

            let mut result;

            // For each entry processed by the sequence buffer, pop it out, write the block, and write the block id
            // FORMAT: Write each sequence block to file

            // This writes out the sequence blocks (SeqBlockCompressed / SBC)
            let start = out_fh.stream_position().unwrap();
            let backoff = Backoff::new();

            let mut output_spins: usize = 0;
            loop {
                result = oq.pop();

                match result {
                    None => {
                        if oq.is_empty() && shutdown.load(Ordering::Relaxed) {
                            log::info!("output_spins: {}", output_spins);
                            break;
                        } else {
                            reader_handle.thread().unpark();
                            output_spins = output_spins.saturating_add(1);
                            backoff.snooze();
                        }
                    }
                    Some((block_id, sb)) => {
                        bincode::encode_into_std_write(sb, &mut out_fh, bincode_config)
                            .expect("Unable to write to bincode output");
                        // log::info!("Writer wrote block {}", block_id);

                        block_locs.push((block_id, pos));

                        pos = out_fh
                            .stream_position()
                            .expect("Unable to work with seek API");
                    }
                }
            }

            // Join the FASTA thread (ot block until it can be joined)
            fasta_thread.join().unwrap();

            let end = out_fh.stream_position().unwrap();
            log::info!("DEBUG: Wrote {} bytes of sequence blocks", end - start);
            debug_size.push(("Sequence Blocks".to_string(), (end - start) as usize));

            let start = out_fh.stream_position().unwrap();

            // Block Index
            block_index_pos = Some(
                out_fh
                    .stream_position()
                    .expect("Unable to work with seek API"),
            );

            let mut block_locs_store = U64BlockStore::default();

            // Write the block index to file (We sort so it is ordinal)
            block_locs.sort_by(|a, b| a.0.cmp(&b.0));
            let block_locs: Vec<u64> = block_locs.iter().map(|x| x.1).collect();

            log::info!("DEBUG: Writing {} total blocks", block_locs.len());

            block_locs.into_iter().for_each(|x| {
                block_locs_store.add(x);
            });

            block_locs_store.write_to_buffer(&mut out_fh);

            let end = out_fh.stream_position().unwrap();

            log::info!("DEBUG: Wrote {} bytes of block index", end - start);
            debug_size.push(("Block Index".to_string(), (end - start) as usize));

            reader_handle.thread().unpark();
            let j = reader_handle.join().expect("Unable to join thread");
            seq_locs = Some(j.0);
            headers = Some(j.1);
            ids = Some(j.2);
            ids_string = j.3;
            masking = Some(j.4);

            let start_time = Instant::now();

            let start = out_fh.stream_position().unwrap();

            headers_location = match headers.as_mut().unwrap().write_to_buffer(&mut out_fh) {
                Some(x) => NonZeroU64::new(x),
                None => None,
            };

            let end = out_fh.stream_position().unwrap();

            let end_time = Instant::now();

            debug_size.push(("Headers".to_string(), (end - start) as usize));

            log::info!(
                "DEBUG: Wrote {} bytes of headers in {:#?}",
                end - start,
                end_time - start_time
            );

            let start_time = Instant::now();
            let start = out_fh.stream_position().unwrap();
            ids_location = match ids.as_mut().expect("No ids!").write_to_buffer(&mut out_fh) {
                Some(x) => NonZeroU64::new(x),
                None => None,
            };

            let end = out_fh.stream_position().unwrap();

            let end_time = Instant::now();

            debug_size.push(("IDs".to_string(), (end - start) as usize));
            log::info!(
                "DEBUG: Wrote {} bytes of ids in {:#?}",
                end - start,
                end_time - start_time
            );

            let start_time = Instant::now();
            let start = out_fh.stream_position().unwrap();
            masking_location = match masking.as_mut().unwrap().write_to_buffer(&mut out_fh) {
                Some(x) => NonZeroU64::new(x),
                None => None,
            };

            let end = out_fh.stream_position().unwrap();
            debug_size.push(("Masking".to_string(), (end - start) as usize));
            let end_time = Instant::now();
            log::info!(
                "DEBUG: Wrote {} bytes of masking in {:#?}",
                end - start,
                end_time - start_time
            );

            out_fh.flush().expect("Unable to flush output buffer");

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
            log::info!(
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
enum Work {
    FastaPayload(crate::datatypes::Sequence),
    FastqPayload(crate::datatypes::Sequence),
    Shutdown,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datatypes::*;
    use std::io::Cursor;

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    pub fn test_create_sfasta() {
        init();

        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        let mut out_buf = Box::new(Cursor::new(Vec::new()));

        println!("test_data/test_convert.fasta");
        let mut in_buf = BufReader::new(
            File::open("test_data/test_convert.fasta").expect("Unable to open testing file"),
        );

        let converter = Converter::default().with_threads(6).with_block_size(8192);

        converter.convert_fasta(&mut in_buf, &mut out_buf);

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
        let mut zstd_decompressor = zstd::bulk::Decompressor::new().unwrap();
        zstd_decompressor.include_magicbytes(false).unwrap();

        let b = b.decompress(
            CompressionType::ZSTD,
            2 * 1024 * 1024,
            Some(zstd_decompressor).as_mut(),
        );

        assert!(b.len() == 8192);
    }
}
