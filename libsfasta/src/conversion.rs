//! # Conversion Struct and Functions for FASTA/Q Files, with and without masking, scores, and indexing.
//!
//! Conversion functions for FASTA and FASTQ files. Multithreaded by
//! default.

// todo rename to builder?

// Easy, high-performance conversion functions
use crossbeam::{thread, utils::Backoff};
use humansize::{make_format, DECIMAL};
use needletail::parse_fastx_reader;
use xxhash_rust::xxh3::xxh3_64;

use std::{
    io::{Read, Seek, SeekFrom, Write},
    num::NonZeroU64,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use xbytes::ByteSize;

use crate::{datatypes::*, formats::*};
use libcompression::*;
use libfractaltree::FractalTreeDisk;

/// Main conversion struct
///
/// Follows a builder style pattern, with default settings
/// ```rust
/// use libsfasta::conversion::Converter;
///
/// let mut converter = Converter::default()
///    .with_threads(4);
/// ```
pub struct Converter<W>
where
    W: Write + Seek + Send + Sync + 'static,
{
    index: bool,
    threads: usize,
    dict: bool,
    dict_samples: u64,
    dict_size: u64,
    compression_profile: CompressionProfile,
    sfasta: Sfasta,
    debug_sizes: Vec<(String, usize)>,
    directory_location: u64,

    // Data spaces
    ids_to_locs: Vec<(Arc<Vec<u8>>, u32)>,

    // May not need to be a mutex...
    out_fh: Option<Arc<Mutex<Box<W>>>>,

    /// Builders and such
    output_worker: Option<crate::io::worker::Worker<W>>,
    output_queue: Option<Arc<flume::Sender<OutputBlock>>>,

    compression_workers: Option<Arc<CompressionWorker>>,

    // Data Types
    seqlocs: SeqLocsStoreBuilder,
    headers: StringBlockStoreBuilder,
    ids: StringBlockStoreBuilder,
    masking: MaskingStoreBuilder,
    sequences: BytesBlockStoreBuilder,
    scores: BytesBlockStoreBuilder,
}

/// Default settings for Converter
///
/// * threads: 4
/// * block_size: 512kb
/// * seqlocs_chunk_size: 128k
/// * index: on
/// * masking: off
/// * quality scores: off
/// * compression_type: ZSTD
/// * compression_level: Default (3 for ZSTD)
impl<W> Default for Converter<W>
where
    W: Write + Seek + Send + Sync + 'static,
{
    fn default() -> Self
    {
        Converter {
            threads: 8,
            index: true,
            dict: false,
            dict_samples: 100,
            dict_size: 110 * 1024,
            compression_profile: CompressionProfile::default(),
            sfasta: Sfasta::default().conversion(),
            debug_sizes: Vec::new(),
            directory_location: 0,
            out_fh: None,
            output_worker: None,
            output_queue: None,
            seqlocs: SeqLocsStoreBuilder::default(),
            compression_workers: None,
        }
    }
}

impl<W> Converter<W>
where
    W: Write + Seek + Send + Sync + 'static,
{
    // Builder configuration functions...
    /// Specify a dictionary to use for compression. Untested.
    pub fn with_dict(&mut self, dict_samples: u64, dict_size: u64)
        -> &mut Self
    {
        self.dict = true;
        self.dict_samples = dict_samples;
        self.dict_size = dict_size;
        self
    }

    /// Disable dictionary
    pub fn without_dict(&mut self) -> &mut Self
    {
        self.dict = false;
        self
    }

    /// Enable seq index
    pub fn with_index(&mut self) -> &mut Self
    {
        self.index = true;
        self
    }

    /// Disable index
    pub fn without_index(&mut self) -> &mut Self
    {
        self.index = false;
        self
    }

    /// Set the number of threads to use
    pub fn with_threads(&mut self, threads: usize) -> &mut Self
    {
        assert!(
            threads < u16::MAX as usize,
            "Maximum number of supported threads is u16::MAX"
        );
        self.threads = threads;
        self
    }

    pub fn threads(&self) -> usize
    {
        self.threads
    }

    /// Set the block size for the sequence blocks
    pub fn with_block_size(&mut self, block_size: usize) -> &mut Self
    {
        assert!(
            block_size * 1024 < u32::MAX as usize,
            "Block size must be less than u32::MAX (~4Gb)"
        );

        self.compression_profile.block_size = block_size as u32;
        self
    }

    /// Set the compression type
    pub fn with_compression(
        &mut self,
        ct: CompressionType,
        level: i8,
    ) -> &mut Self
    {
        log::info!("Setting compression to {:?} at level {}", ct, level);
        log::info!("Custom compression profiles often perform better...");
        self.compression_profile = CompressionProfile::set_global(ct, level);
        self
    }

    /// Set the compression profile
    pub fn with_compression_profile(
        &mut self,
        profile: CompressionProfile,
    ) -> &mut Self
    {
        self.compression_profile = profile;
        self
    }

    /// Write the headers for the SFASTA file, and return the location
    /// of the headers (so they can be updated at the end)
    fn write_headers(&mut self) -> u64
    {
        // IMPORTANT: Headers must ALWAYS be fixed int encoding
        let bincode_config = crate::BINCODE_CONFIG.with_fixed_int_encoding();

        let mut out_fh = self.out_fh.as_mut().unwrap().lock().unwrap();

        let loc = out_fh
            .stream_position()
            .expect("Unable to work with seek API");

        out_fh
            .write_all("sfasta".as_bytes())
            .expect("Unable to write 'sfasta' to output");

        // Write the directory

        // Write the version
        bincode::encode_into_std_write(
            self.sfasta.version,
            &mut *out_fh,
            bincode_config,
        )
        .expect("Unable to write directory to file");

        let dir: DirectoryOnDisk = self.sfasta.directory.clone().into();
        bincode::encode_into_std_write(dir, &mut *out_fh, bincode_config);

        loc
    }

    pub fn init(&mut self, out_fh: Box<W>)
    where
        W: WriteAndSeek + 'static + Send + Sync,
    {
        self.out_fh = Some(Arc::new(Mutex::new(out_fh)));

        // Set dummy values for the directory
        self.sfasta.directory.dummy();

        if self.sfasta.metadata.is_none() {
            self.sfasta.metadata = Some(Metadata::default());
        }

        let epoch_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();

        self.sfasta.metadata.as_mut().unwrap().date_created =
            epoch_time.as_secs();

        // Store the location of the directory so we can update it later...
        // It's right after the SFASTA and version identifier...
        let directory_location = self.write_headers();
        self.directory_location = directory_location;

        // Start the output I/O...
        let output_buffer = Arc::clone(&self.out_fh.as_ref().unwrap());
        let mut output_worker = crate::io::worker::Worker::new(output_buffer)
            .with_buffer_size(self.threads as usize * 8);

        output_worker.start();
        let output_queue = output_worker.get_queue();
        self.output_worker = Some(output_worker);
        self.output_queue = Some(Arc::clone(&output_queue));

        let mut compression_workers = CompressionWorker::new()
            .with_buffer_size(self.threads as usize * 2)
            .with_threads(self.threads as u16)
            .with_output_queue(output_queue);

        compression_workers.start();
        let compression_workers = Arc::new(compression_workers);
        self.compression_workers = Some(compression_workers);

        let mut seqlocs = SeqLocsStoreBuilder::default();
        seqlocs =
            seqlocs.with_compression(self.compression_profile.seqlocs.clone());
        self.seqlocs = seqlocs;

        let compression_workers_thread = Arc::clone(self.compression_workers.as_ref().unwrap());

        let mut headers = StringBlockStoreBuilder::default()
            .with_block_size(self.compression_profile.block_size as usize)
            .with_compression(self.compression_profile.data.headers.clone())
            .with_tree_compression(
                self.compression_profile.index.headers.clone(),
            )
            .with_compression_worker(Arc::clone(&compression_workers_thread));

        let mut ids = StringBlockStoreBuilder::default()
            .with_block_size(self.compression_profile.block_size as usize)
            .with_compression(self.compression_profile.data.ids.clone())
            .with_tree_compression(self.compression_profile.index.ids.clone())
            .with_compression_worker(Arc::clone(&compression_workers_thread));

        let mut masking = MaskingStoreBuilder::default()
            .with_compression_worker(Arc::clone(&compression_workers_thread))
            .with_compression(self.compression_profile.data.masking.clone())
            .with_tree_compression(
                self.compression_profile.index.masking.clone(),
            )
            .with_block_size(self.compression_profile.block_size as usize);

        let mut sequences = BytesBlockStoreBuilder::default()
            .with_block_size(self.compression_profile.block_size as usize)
            .with_compression(self.compression_profile.data.sequence.clone())
            .with_compression_worker(Arc::clone(&compression_workers_thread));

        let mut scores = BytesBlockStoreBuilder::default()
            .with_block_size(self.compression_profile.block_size as usize)
            .with_compression(self.compression_profile.data.quality.clone())
            .with_tree_compression(
                self.compression_profile.index.quality.clone(),
            )
            .with_compression_worker(Arc::clone(&compression_workers_thread));

        if self.dict {
            ids = ids
                .with_dict()
                .with_dict_samples(self.dict_samples)
                .with_dict_size(self.dict_size);
            headers = headers
                .with_dict()
                .with_dict_samples(self.dict_samples)
                .with_dict_size(self.dict_size);
            masking = masking
                .with_dict()
                .with_dict_samples(self.dict_samples)
                .with_dict_size(self.dict_size);
            sequences = sequences
                .with_dict()
                .with_dict_samples(self.dict_samples)
                .with_dict_size(self.dict_size);
            scores = scores
                .with_dict()
                .with_dict_samples(self.dict_samples)
                .with_dict_size(self.dict_size);
        }

        self.headers = headers;
        self.ids = ids;
        self.masking = masking;
        self.sequences = sequences;
        self.scores = scores;

    }

    pub fn finish(&mut self, out_fh: &mut Box<dyn WriteAndSeek>)
    {
        // todo anything else?
        out_fh
            .seek(SeekFrom::Start(self.directory_location))
            .expect("Unable to seek to start of the file");

        self.write_headers();
    }

    /// Main conversion function for FASTA/Q files
    pub fn convert<'convert, R>(
        mut self,
        in_buf: &mut R,
        mut out_fh: Box<W>,
    )
    where
        R: Read + Send + 'convert,
    {
        self.init(out_fh);

        log::info!("Writing sequences start...",);

        let out_buffer = Arc::new(Mutex::new(out_fh));

        self.process(
            in_buf,
            Arc::clone(&out_buffer),
            self.compression_profile.block_size * 1024,
        );

        let mut fractaltree_pos = 0;

        let mut seqlocs_location = 0;

        let mut out_buffer_thread = out_buffer.lock().unwrap();

        // Build the index in another thread...
        thread::scope(|s| {
            let mut indexer = libfractaltree::FractalTreeBuild::new(512, 2048);

            let index_handle = Some(s.spawn(|_| {
                for (id, loc) in self.ids_to_locs.into_iter() {
                    let id = xxh3_64(&id);
                    // let loc = loc.load(Ordering::SeqCst);
                    indexer.insert(id as u32, loc);
                }
                indexer.flush_all();

                indexer
            }));

            let formatter = make_format(DECIMAL);

            let start = out_buffer_thread.stream_position().unwrap();

            // let mut seqlocs = seqlocs.with_dict();

            log::trace!("Storing seqlocs");
            seqlocs_location =
                self.seqlocs.write_to_buffer(&mut *out_buffer_thread).unwrap();

            log::info!(
                "SeqLocs Size: {} {} {}",
                formatter(out_buffer_thread.stream_position().unwrap() - start),
                out_buffer_thread.stream_position().unwrap(),
                start
            );

            if self.index {
                let index = index_handle.unwrap().join().unwrap();
                let start = out_buffer_thread.stream_position().unwrap();

                // This points from u32 hash to u32 location (seqloc).
                // This is done so the seqlocs can be in insertion order

                let mut index: FractalTreeDisk<u32, u32> = index.into();

                index
                    .set_compression(self.compression_profile.id_index.clone());

                fractaltree_pos = index
                    .write_to_buffer(&mut *out_buffer_thread)
                    .expect("Unable to write index to file");

                let end = out_buffer_thread.stream_position().unwrap();

                log::info!("Index Size: {}", formatter(end - start));
                self.debug_size.push(("index".to_string(), (end - start) as usize));
            }
        })
        .expect("Error");

        drop(out_buffer_thread);

        self.sfasta.directory.seqlocs_loc = NonZeroU64::new(seqlocs_location);
        self.sfasta.directory.index_loc = NonZeroU64::new(fractaltree_pos);
        self.sfasta.directory.headers_loc = headers_location;
        self.sfasta.directory.ids_loc = ids_location;
        self.sfasta.directory.masking_loc = masking_location;
        self.sfasta.directory.sequences_loc = sequences_location;
        self.sfasta.directory.scores_loc = scores_location;

        // Go to the beginning, and write the location of the index

        let mut out_buffer_lock = out_buffer.lock().unwrap();
        out_buffer_lock
            .seek(SeekFrom::Start(directory_location))
            .expect("Unable to rewind to start of the file");

        // Here we re-write the directory information at the start of the
        // file, allowing for easy jumps to important areas while
        // keeping everything in a single file

        let start = out_buffer_lock.stream_position().unwrap();

        self.write_headers(&mut *out_buffer_lock, &sfasta);

        let end = out_buffer_lock.stream_position().unwrap();
        debug_size.push(("sfasta headers".to_string(), (end - start) as usize));

        log::info!("DEBUG: {:?}", debug_size);

        out_buffer_lock
            .flush()
            .expect("Unable to flush output file");

        drop(out_buffer_lock);

        // Pull out of Arc and Mutex
        match Arc::try_unwrap(out_buffer) {
            Ok(x) => x.into_inner().unwrap(),
            Err(_) => panic!("Unable to unwrap out_buffer"),
        }
    }

    /// Process buffer and write to output file
    pub fn process<'convert, R>(
        &self,
        in_buf: &mut R,
        out_fh: Arc<Mutex<Box<W>>>,
        block_size: u32,
    ) -> (
        Vec<(std::sync::Arc<Vec<u8>>, u32)>, // todo: cow?
    )
    where
        R: Read + Send + 'convert,
    {
        let mut reader = parse_fastx_reader(in_buf).unwrap();

        while let Some(x) = reader.next() {
            match x {
                Ok(x) => {
                    let seq = x.seq();
                    let mut id = x.id().splitn(2, |x| *x == b' ');
                    // Split at first space
                    let seqid = id.next().unwrap();
                    let seqheader = id.next();

                    // Scores
                    let score_locs = if let Some(quals) = x.qual() {
                        let quals = quals.to_vec();
                        // Makes it larger for some reason...
                        // libfractaltree::disk::delta_encode(&mut quals);
                        let loc = self.scores.add(quals);
                        loc.unwrap()
                    } else {
                        vec![]
                    };

                    let masking_locs = self.masking.add(seq.to_vec());

                    // Capitalize and add to sequences
                    let mut seq = seq.to_vec();
                    seq.make_ascii_uppercase();

                    let loc = self.sequences.add(seq);
                    let sequence_locs = loc.unwrap();

                    let myid = std::sync::Arc::new(seqid.to_vec());
                    let id_locs = self.ids.add(myid.to_vec());

                    let headers_loc = if let Some(x) = seqheader {
                        self.headers.add(x.to_vec())
                    } else {
                        vec![]
                    };

                    let mut seqloc = SeqLoc::new();
                    seqloc.add_locs(
                        &id_locs,
                        &sequence_locs,
                        &masking_locs,
                        &score_locs,
                        &headers_loc,
                        &vec![],
                        &vec![],
                    );

                    let loc = self.seqlocs.add_to_index(seqloc);

                    // ids_string.push(Arc::clone(&myid));
                    self.ids_to_locs.push((myid, loc));
                }
                Err(x) => {
                    log::error!("Error parsing sequence: {:?}", x);
                }
            }
        }

        log::info!("Finished reading sequences");

        // let mut headers = headers.join().unwrap();
        // let mut ids = ids.join().unwrap();
        // let mut masking = masking.join().unwrap();
        // let mut sequences = sequences.join().unwrap();
        // let mut scores = scores.join().unwrap();

        // Wait for all the workers to finish...
        self.headers.finalize();
        self.ids.finalize();
        self.masking
            .finalize()
            .expect("Unable to finalize masking store");
        self.sequences
            .finalize()
            .expect("Unable to finalize sequences store");
        self.scores.finalize().expect("Unable to finalize scores store");

        let backoff = Backoff::new();
        while self.compression_workers.as_ref().unwrap().len() > 0 || self.output_worker.as_ref().unwrap().len() > 0 {
            backoff.snooze();
            if backoff.is_completed() {
                std::thread::sleep(std::time::Duration::from_millis(20));
                log::debug!(
                    "Compression Length: {} Output Worker Length: {}",
                    self.compression_workers.as_ref().unwrap().len(),
                    self.output_worker.as_ref().unwrap().len()
                );
                backoff.reset();
            }
        }

        self.output_worker.as_mut().unwrap().shutdown();

        let formatter = make_format(DECIMAL);
        let mut out_buffer = out_fh.lock().unwrap();

        let start = out_buffer.stream_position().unwrap();

        // --------------------------- Headers

        log::trace!("Writing headers");

        match self.headers.write_block_locations(&mut *out_buffer)
        {
            Ok(_) => (),
            Err(x) => match x {
                BlockStoreError::Empty => (),
                _ => panic!("Error writing headers: {:?}", x),
            },
        };

        let end = out_buffer.stream_position().unwrap();
        log::debug!("Headers Fractal Tree Size: {}", formatter(end - start));

        log::info!(
            "Headers Compression Ratio: {:.2}% ({} -> {})",
            (self.headers.inner.compressed_size as f64
                / self.headers.inner.data_size as f64
                * 100.0),
            ByteSize::from_bytes(
                self.headers.inner.data_size as u64
            )
            .to_string(),
            ByteSize::from_bytes(
                self.headers.inner.compressed_size as u64
            )
            .to_string()
        );

        // --------------------------- IDs

        log::trace!("Writing IDs");

        let start = out_buffer.stream_position().unwrap();
        match self.ids.write_block_locations(&mut *out_buffer) {
            Ok(_) => (),
            Err(x) => match x {
                BlockStoreError::Empty => (),
                _ => panic!("Error writing ids: {:?}", x),
            },
        };
        let end = out_buffer.stream_position().unwrap();
        log::debug!("IDs Fractal Tree Size: {}", formatter(end - start));

        log::info!(
            "IDs Compression Ratio: {:.2}% ({} -> {}) - Avg Compressed Block Size: {}",
                (self.ids.inner.compressed_size as f64
                    / self.ids.inner.data_size as f64
                    * 100.0),
            ByteSize::from_bytes(self.ids.inner.data_size as u64)
                .to_string(),
            ByteSize::from_bytes(
                self.ids.inner.compressed_size as u64
            )
            .to_string(),
            ByteSize::from_bytes(
                self.ids.inner.block_data.iter().map(|x| x.1.load(std::sync::atomic::Ordering::SeqCst)).sum::<u64>() as u64 / ids.as_ref().unwrap().inner.block_data.len() as u64
            )
        );

        // --------------------------- Masking

        // todo make sure we are handling empties properly!

        let start = out_buffer.stream_position().unwrap();
        match self.masking.write_block_locations(&mut *out_buffer)
        {
            Ok(_) => {
                let end = out_buffer.stream_position().unwrap();
                log::debug!(
                    "Masking Fractal Tree Size: {}",
                    formatter(end - start)
                );

                log::info!(
                    "Masking Compression Ratio: {:.2}% ({} -> {})  - Avg Compressed Block Size: {}",
                        (self.masking.inner.compressed_size as f64
                            / self.masking.inner.data_size as f64
                            * 100.0),
                    ByteSize::from_bytes(
                        self.masking.inner.data_size as u64
                    )
                    .to_string(),
                    ByteSize::from_bytes(
                        self.masking.inner.compressed_size as u64
                    )
                    .to_string(),
                    ByteSize::from_bytes(
                        self.masking.inner.block_data.iter().map(|x| x.1.load(std::sync::atomic::Ordering::SeqCst)).sum::<u64>() as u64 / masking.as_ref().unwrap().inner.block_data.len() as u64
                    )
                );
    
            },
            Err(x) => match x {
                BlockStoreError::Empty => (),
                _ => panic!("Error writing masking: {:?}", x),
            },
        };
        
        // --------------------------- Sequences

        let start = out_buffer.stream_position().unwrap();
        match self.sequences.write_block_locations(&mut *out_buffer) {
                Ok(_) => {
                    let end = out_buffer.stream_position().unwrap();
                    log::debug!("Sequences Fractal Tree Size: {}", formatter(end - start));
            
                    log::info!(
                        "Sequences Compression Ratio: {:.2}% ({} -> {}) - Avg Compressed Block Size: {}",
                            (self.sequences.compressed_size as f64
                                / self.sequences.data_size as f64
                                * 100.0),
                        ByteSize::from_bytes(self.sequences.data_size as u64)
                            .to_string(),
                        ByteSize::from_bytes(
                            self.sequences.compressed_size as u64
                        )
                        .to_string(),
                        ByteSize::from_bytes(
                            self.sequences.block_data.iter().map(|x| x.1.load(std::sync::atomic::Ordering::SeqCst)).sum::<u64>() as u64 / sequences.as_ref().unwrap().block_data.len() as u64
                        )
                    );
                },
                Err(x) => match x {
                    BlockStoreError::Empty => (),
                    _ => panic!("Error writing sequences: {:?}", x),
                },
            };


        // --------------------------- Scores

        let start = out_buffer.stream_position().unwrap();
        match self.scores.write_block_locations(&mut *out_buffer) {
            Ok(_) => {
                let end = out_buffer.stream_position().unwrap();
                log::debug!("Scores Fractal Tree Size: {}", formatter(end - start));

                log::info!(
                    "Scores Compression Ratio: {:.2}% ({} -> {}) - Avg Compressed Block Size: {}",
                        (self.scores.compressed_size as f64
                        / self.scores.data_size as f64
                        * 100.0),
                ByteSize::from_bytes(self.scores.data_size as u64)
                    .to_string(),
                ByteSize::from_bytes(
                    self.scores.compressed_size as u64
                )
                .to_string(),
                ByteSize::from_bytes(
                    self.scores.block_data.iter().map(|x| x.1.load(std::sync::atomic::Ordering::SeqCst)).sum::<u64>() as u64 / scores.as_ref().unwrap().block_data.len() as u64
                )
            );
            }
            Err(x) => match x {
                BlockStoreError::Empty => (),
                _ => panic!("Error writing scores: {:?}", x),
            },
        };

        // --- finished

        // Write the headers for each store...
        if !self.headers.is_empty() {
            self.sfasta.directory.headers_loc =
                NonZeroU64::new(out_buffer.stream_position().unwrap());
            self.headers.write_header(self.sfasta.directory.headers_loc.unwrap().into(), &mut *out_buffer);
        } else {
            self.sfasta.directory.headers_loc = None;
        }

        if ids.is_some() {
            let x = ids.as_mut().unwrap();
            ids_location =
                NonZeroU64::new(out_buffer.stream_position().unwrap());
            x.write_header(ids_location.unwrap().get(), &mut *out_buffer);
        } else {
            ids_location = None;
        }

        if masking.is_some() {
            let x = masking.as_mut().unwrap();
            masking_location =
                NonZeroU64::new(out_buffer.stream_position().unwrap());
            x.write_header(masking_location.unwrap().get(), &mut *out_buffer);
        } else {
            masking_location = None;
        }

        if sequences.is_some() {
            let x = sequences.as_mut().unwrap();
            sequences_location =
                NonZeroU64::new(out_buffer.stream_position().unwrap());
            x.write_header(sequences_location.unwrap().get(), &mut *out_buffer);
        } else {
            sequences_location = None;
        }

        if scores.is_some() {
            let x = scores.as_mut().unwrap();
            scores_location =
                NonZeroU64::new(out_buffer.stream_position().unwrap());
            x.write_header(scores_location.unwrap().get(), &mut *out_buffer);
        } else {
            scores_location = None;
        }

        log::debug!("Conversion Process function complete");

        // let seqlocs = seqlocs.join().unwrap();

        out_buffer.flush().expect("Unable to flush output buffer");

        (
            seqlocs,
            headers_location,
            ids_location,
            masking_location,
            sequences_location,
            scores_location,
            ids_to_locs,
        )
    }
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
        let mut in_buf = std::io::BufReader::new(
            File::open("test_data/test_convert.fasta")
                .expect("Unable to open testing file"),
        );

        let mut converter = Converter::default();
        converter.with_threads(6).with_block_size(8);

        assert!(converter.threads() == 6);

        let mut out_buf = converter.convert(&mut in_buf, out_buf);

        if let Err(x) = out_buf.seek(SeekFrom::Start(0)) {
            panic!("Unable to seek to start of file, {:#?}", x)
        };

        let mut sfasta_marker: [u8; 6] = [0; 6];
        out_buf
            .read_exact(&mut sfasta_marker)
            .expect("Unable to read SFASTA Marker");
        assert!(sfasta_marker == "sfasta".as_bytes());

        let _version: u64 =
            bincode::decode_from_std_read(&mut out_buf, bincode_config)
                .unwrap();

        let directory: DirectoryOnDisk =
            bincode::decode_from_std_read(&mut out_buf, bincode_config)
                .unwrap();

        let _dir = directory;

        // let _metadata: Metadata =
        // bincode::decode_from_std_read(&mut out_buf, bincode_config)
        // .unwrap();
        //

        // TODO: Add more tests
    }
}
