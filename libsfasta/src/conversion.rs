//! # Conversion Struct and Functions for FASTA/Q Files, with and without masking, scores, and indexing.
//!
//! Conversion functions for FASTA and FASTQ files. Multithreaded by default.

// Easy, high-performance conversion functions
use crossbeam::{queue::ArrayQueue, thread, utils::Backoff};
use lz4_flex::block;
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
    pub block_size: u64,
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
            block_size: 512 * 1024, // 4Mb
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

        self.block_size = block_size as u64;
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

        assert!(self.block_size < u32::MAX as u64);

        let mut sfasta = Sfasta::default().block_size(self.block_size as u32);

        // Store masks as series of 0s and 1s... Vec<bool>
        // Compression seems to take care of the size. bitvec! and vec! seem to have similar
        // performance and on-disk storage requirements
        if self.masking {
            sfasta = sfasta.with_masking();
        }

        sfasta.parameters.compression_type = self.compression_type;
        sfasta.parameters.compression_dict = self.dict.clone();
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
            self.process(in_buf, Arc::clone(&out_buffer), self.block_size);

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
                log::debug!("Index pos: {}", fractaltree_pos);

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
        block_size: u64,
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
        let mut output_worker = crate::io::worker::Worker::new(output_buffer).with_buffer_size(1024);
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
        let headers_location;
        let ids_location;
        let masking_location;
        let sequences_location;

        let mut ids_string = Vec::new();
        let mut ids_to_locs = Vec::new();

        // TODO: Multithread this part -- maybe, now with compression logic into a threadpool maybe not...
        // Idea: split into queue's for headers, IDs, sequence, etc....
        // Thread that handles the heavy lifting of the incoming Seq structs
        // TODO: Set compression stuff here...
        // And batch this part...

        let compression_workers_thread = Arc::clone(&compression_workers);

        let mut headers = StringBlockStoreBuilder::default()
            .with_block_size(block_size as usize)
            .with_compression_worker(Arc::clone(&compression_workers_thread));

        let mut seqlocs = SeqLocsStoreBuilder::default();

        let mut ids = StringBlockStoreBuilder::default()
            .with_block_size(block_size as usize)
            .with_compression_worker(Arc::clone(&compression_workers_thread));

        let mut masking = MaskingStoreBuilder::default()
            .with_compression_worker(Arc::clone(&compression_workers_thread))
            .with_block_size(block_size as usize);

        let mut sequences = SequenceBlockStoreBuilder::default()
            .with_block_size(block_size as usize)
            .with_compression_worker(Arc::clone(&compression_workers_thread));

        let mut masking_times = Vec::new();
        let mut seq_add_times = Vec::new();
        let mut headers_time = Vec::new();
        let mut ids_time = Vec::new();
        let mut seqloc_time = Vec::new();
        let mut seqlocs_time = Vec::new();

        let mut reader = parse_fastx_reader(in_buf).unwrap();

        while let Some(x) = reader.next() {
            match x {
                Ok(x) => {
                    let mut seq = x.seq();
                    let mut id = x.id().split(|x| *x == b' ');
                    // Split at first space
                    let seqid = id.next().unwrap();
                    let seqheader = id.next();

                    let start_time = std::time::Instant::now();
                    let masking_locs = masking.add_masking(&seq);

                    let end_time = std::time::Instant::now();
                    masking_times.push(end_time - start_time);

                    // Capitalize and add to sequences
                    let start_time = std::time::Instant::now();
                    let mut sequence_locs = Vec::new();
                    seq.to_mut().make_ascii_uppercase();
                    let loc = sequences.add(&mut seq);
                    sequence_locs.extend(loc);

                    let end_time = std::time::Instant::now();
                    seq_add_times.push(end_time - start_time);

                    let start_time = std::time::Instant::now();
                    let myid = std::sync::Arc::new(seqid.to_vec());
                    let idloc = ids.add(&(*myid));
                    let end_time = std::time::Instant::now();
                    ids_time.push(end_time - start_time);

                    let start_time = std::time::Instant::now();
                    let mut headers_loc = Vec::new();
                    if let Some(x) = seqheader {
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

                    let loc = seqlocs.add_to_index(seqloc);
                    ids_string.push(Arc::clone(&myid));
                    ids_to_locs.push((myid, loc));
                    let end_time = std::time::Instant::now();
                    seqlocs_time.push(end_time - start_time);
                }
                Err(x) => {
                    log::error!("Error parsing sequence: {:?}", x);
                }
            }
        }

        // Print out the mean and range of the times...
        let mean = |x: &Vec<std::time::Duration>| {
            let sum: std::time::Duration = x.iter().sum();
            sum / x.len() as u32
        };

        let range = |x: &Vec<std::time::Duration>| {
            let min = x.iter().min().unwrap();
            let max = x.iter().max().unwrap();
            (*min, *max)
        };

        log::info!(
            "Masking mean: {:?} range: {:?}",
            mean(&masking_times),
            range(&masking_times)
        );
        log::info!(
            "Seq add mean: {:?} range: {:?}",
            mean(&seq_add_times),
            range(&seq_add_times)
        );
        log::info!(
            "Headers mean: {:?} range: {:?}",
            mean(&headers_time),
            range(&headers_time)
        );
        log::info!("IDs mean: {:?} range: {:?}", mean(&ids_time), range(&ids_time));
        log::info!("SeqLoc mean: {:?} range: {:?}", mean(&seqloc_time), range(&seqloc_time));
        log::info!(
            "SeqLocs mean: {:?} range: {:?}",
            mean(&seqlocs_time),
            range(&seqlocs_time)
        );

        headers.finalize();
        ids.finalize();
        masking.finalize();
        sequences.finalize();

        let backoff = Backoff::new();
        while compression_workers.len() > 0 || output_worker.len() > 0 {
            backoff.snooze();
            if backoff.is_completed() {
                std::thread::sleep(std::time::Duration::from_millis(20));
                log::debug!(
                    "Compression Length: {} Output Worker Length: {}",
                    compression_workers.len(),
                    output_worker.len()
                );
                backoff.reset();
            }
        }

        output_worker.shutdown();

        let mut out_buffer = out_fh.lock().unwrap();

        let mut headers = match headers.write_block_locations(&mut *out_buffer) {
            Ok(x) => Some(headers),
            Err(x) => match x {
                BlockStoreError::Empty => None,
                _ => panic!("Error writing headers: {:?}", x),
            },
        };

        let mut ids = match ids.write_block_locations(&mut *out_buffer) {
            Ok(x) => Some(ids),
            Err(x) => match x {
                BlockStoreError::Empty => None,
                _ => panic!("Error writing ids: {:?}", x),
            },
        };

        let mut masking = match masking.write_block_locations(&mut *out_buffer) {
            Ok(x) => Some(masking),
            Err(x) => match x {
                BlockStoreError::Empty => None,
                _ => panic!("Error writing masking: {:?}", x),
            },
        };

        let mut sequences = match sequences.write_block_locations(&mut *out_buffer) {
            Ok(x) => Some(sequences),
            Err(x) => match x {
                BlockStoreError::Empty => None,
                _ => panic!("Error writing sequences: {:?}", x),
            },
        };

        // Write the headers for each store...
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

        (
            seqlocs,
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
