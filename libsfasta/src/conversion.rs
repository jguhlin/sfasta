// Easy, high-performance conversion functions
use crossbeam::queue::ArrayQueue;
use crossbeam::thread;
use crossbeam::utils::Backoff;

use std::fs::{metadata, File};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::num::NonZeroU64;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::compression_stream_buffer::{CompressionStreamBuffer, CompressionStreamBufferConfig};
use crate::datatypes::*;
use crate::dual_level_index::*;
use crate::formats::*;
use crate::utils::*;
use crate::CompressionType;

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

impl Default for Converter {
    fn default() -> Self {
        Converter {
            threads: 4,
            block_size: 4 * 1024 * 1024,    // 1Mb
            seqlocs_chunk_size: 128 * 1024, // 128k
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

    pub fn with_compression_level(mut self, level: i8) -> Self {
        self.compression_level = Some(level);
        self
    }

    pub fn write_headers<W>(&self, mut out_fh: &mut Box<W>, sfasta: &Sfasta) -> u64
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
    pub fn convert_fasta<'convert, W, R>(self, mut in_buf: R, out_fh: &mut Box<W>)
    where
        W: WriteAndSeek + 'static,
        R: Read + Send + 'convert,
    {
        let fn_start_time = std::time::Instant::now();

        let mut debug_size: Vec<(String, usize)> = Vec::new();

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
            out_fh.seek(SeekFrom::Current(0)).unwrap()
        );

        let start = out_fh.seek(SeekFrom::Current(0)).unwrap();

        // Write sequences
        let mut sb_config = CompressionStreamBufferConfig::default()
            .with_block_size(self.block_size as u32)
            .with_compression_type(self.compression_type)
            .with_threads(self.threads as u16); // Effectively # of compression threads

        if self.compression_level.is_some() {
            sb_config = sb_config.with_compression_level(self.compression_level.unwrap());
        }

        if let Some(dict) = self.dict {
            sb_config = sb_config.with_compression_dict(dict);
        }

        let start_time = std::time::Instant::now();

        // Function returns:
        // Vec<(String, Location)>
        // block_index_pos
        let (ids, mut seqlocs, block_index_pos, headers_location, ids_location, masking_location) =
            write_fasta_sequence(sb_config, &mut in_buf, out_fh, &mut debug_size);

        let end_time = std::time::Instant::now();

        log::info!(
            "Write Fasta Sequence write time: {:?}",
            end_time - start_time
        );

        let end = out_fh.seek(SeekFrom::Current(0)).unwrap();

        debug_size.push(("sequences".to_string(), (end - start) as usize));

        log::info!(
            "Writing sequences finished... {}",
            out_fh.seek(SeekFrom::Current(0)).unwrap()
        );

        // TODO: Here is where we would write out the scores...
        // ... but this fn is only for FASTA right now...

        // TODO: Here is where we would write out the masking...

        // TODO: Here is where we would write out the Seqinfo stream (if it's decided to do it)

        // TODO: Support for Index32 (and even smaller! What if only 1 or 2 sequences?)
        let mut indexer =
            crate::dual_level_index::DualIndexBuilder::with_capacity(seqlocs.index_len());

        let mut out_buf = BufWriter::with_capacity(256 * 1024, out_fh);

        let start_time = std::time::Instant::now();

        let end_time = std::time::Instant::now();

        log::info!("Create SeqLocs Struct time: {:?}", end_time - start_time);

        // The index points to the location of the Location structs.
        // Location blocks are chunked into SEQLOCS_CHUNK_SIZE
        // So index.get(id) --> integer position of the location struct
        // Which will be in integer position / SEQLOCS_CHUNK_SIZE chunk offset
        // This will then point to the different location blocks where the sequence is...
        //
        // TODO: Optional index can probably be handled better...

        let mut dual_index_pos = 0;

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
            log::info!(
                "Writing SeqLocs to file. {}",
                out_buf.seek(SeekFrom::Current(0)).unwrap()
            );

            let start = out_buf.seek(SeekFrom::Current(0)).unwrap();

            let start_time = std::time::Instant::now();

            seqlocs_location = seqlocs.write_to_buffer(&mut out_buf);
            log::info!(
                "Writing SeqLocs to file: COMPLETE. {}",
                out_buf.seek(SeekFrom::Current(0)).unwrap()
            );

            let end_time = std::time::Instant::now();
            log::info!("SeqLocs write time: {:?}", end_time - start_time);

            let end = out_buf.seek(SeekFrom::Current(0)).unwrap();
            debug_size.push(("seqlocs".to_string(), (end - start) as usize));

            if self.index {
                dual_index_pos = out_buf
                    .seek(SeekFrom::Current(0))
                    .expect("Unable to work with seek API");

                let mut index = index_handle.unwrap().join().unwrap();
                log::info!(
                    "Writing index to file. {}",
                    out_buf.seek(SeekFrom::Current(0)).unwrap()
                );

                let start = out_buf.seek(SeekFrom::Current(0)).unwrap();

                let start_time = std::time::Instant::now();
                index.write_to_buffer(&mut out_buf);
                let end_time = std::time::Instant::now();
                log::info!("Index write time: {:?}", end_time - start_time);

                let end = out_buf.seek(SeekFrom::Current(0)).unwrap();
                debug_size.push(("index".to_string(), (end - start) as usize));

                log::info!(
                    "Writing index to file: COMPLETE. {}",
                    out_buf.seek(SeekFrom::Current(0)).unwrap()
                );
            }
        })
        .expect("Error");

        sfasta.directory.seqlocs_loc = NonZeroU64::new(seqlocs_location);
        sfasta.directory.index_loc = NonZeroU64::new(dual_index_pos);
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

        log::info!("Directory write time: {:?}", end_time - start_time);

        log::info!("DEBUG: {:?}", debug_size);

        out_buf.flush().expect("Unable to flush output file");

        let fn_end_time = std::time::Instant::now();
        log::info!("Conversion time: {:?}", fn_end_time - fn_start_time);
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
    FastaPayload(crate::datatypes::Sequence),
    FastqPayload(crate::datatypes::Sequence),
    Shutdown,
}

pub fn write_fasta_sequence<'convert, W, R>(
    sb_config: CompressionStreamBufferConfig,
    in_buf: &mut R,
    mut out_fh: &mut Box<W>,
    mut debug_size: &mut Vec<(String, usize)>,
) -> (
    Vec<std::sync::Arc<String>>,
    SeqLocs<'convert>,
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

    let mut sb = CompressionStreamBuffer::from_config(sb_config);

    // Get a handle for the output queue from the sequence compressor
    let oq = sb.get_output_queue();

    // Get a handle to the shutdown flag...
    let shutdown = sb.get_shutdown_flag();

    let mut seq_locs: Option<SeqLocs> = None;
    let mut block_index_pos = None;
    let mut headers = None;
    let mut headers_location = None;
    let mut ids = None;
    let mut ids_location = None;
    let mut ids_string = Vec::new();
    let mut masking = None;
    let mut masking_location = None;

    let fasta_queue: Arc<ArrayQueue<Work>> = std::sync::Arc::new(ArrayQueue::new(8192 * 4));

    let fasta_queue_in = Arc::clone(&fasta_queue);
    let fasta_queue_out = Arc::clone(&fasta_queue);

    // Pop to a new thread that pushes FASTA sequences into the sequence buffer...
    thread::scope(|s| {
        let fasta_thread = s.spawn(|_| {
            let mut fasta_thread_spins: usize = 0;
            // Convert reader into buffered reader then into the Fasta struct (and iterator)
            let mut in_buf_reader = BufReader::new(in_buf);
            let fasta = Fasta::from_buffer(&mut in_buf_reader);

            let backoff = Backoff::new();

            for x in fasta {
                if x.is_err() {
                    while fasta_queue_in.push(Work::Shutdown).is_err() {
                        backoff.snooze();
                    }

                    log::error!("Error reading FASTA file: {:?}", x);
                    panic!("Error reading FASTA file: {:?}", x);
                }

                let mut d = Work::FastaPayload(x.unwrap());
                while let Err(z) = fasta_queue_in.push(d) {
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

            while fasta_queue_in.push(Work::Shutdown).is_err() {
                backoff.snooze();
            }

            log::info!("fasta_thread_spins: {}", fasta_thread_spins);
        });

        let fasta_thread_clone = fasta_thread.thread().clone();
        let mut out_buf = BufWriter::with_capacity(256 * 1024, &mut out_fh);

        // let mut seq_locs = Arc::new(Mutex::new(Vec::with_capacity(1024)));

        // TODO: Multithread this part
        let reader_handle = s.spawn(move |_| {
            sb.initialize();

            let mut headers = StringBlockStore::default().with_block_size(1024 * 1024);
            let mut seqlocs = SeqLocs::default();
            let mut ids = StringBlockStore::default().with_block_size(512 * 1024);
            let mut ids_string = Vec::new();
            let mut masking = Masking::default();

            // For each Sequence in the fasta file, make it upper case (masking is stored separately)
            // Add the sequence, get the SeqLocs and store them in Location struct
            // And store that in seq_locs Vec...

            let backoff = Backoff::new();

            let mut masking_time = std::time::Duration::new(0, 0);
            let mut adding_time = std::time::Duration::new(0, 0);
            let mut seq_loc_time = std::time::Duration::new(0, 0);

            let mut fasta_queue_spins: usize = 0;
            loop {
                backoff.reset();
                fasta_thread_clone.unpark();
                match fasta_queue_out.pop() {
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
                        if let Some(x) = seqheader {
                            let x = seqlocs.add_locs(&headers.add(x));
                            location.headers = Some((x.0, x.1 as u8));
                        }
                        let x = seqlocs.add_locs(&idloc);
                        location.ids = Some((x.0, x.1 as u8));
                        location.sequence = Some(seqlocs.add_locs(&loc));
                        seq_loc_time += now.elapsed();
                        seqlocs.add_to_index(location);
                    }
                    Some(Work::FastqPayload(_)) => panic!("Received FASTQ payload in FASTA thread"),
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
        let mut pos = out_buf
            .seek(SeekFrom::Current(0))
            .expect("Unable to work with seek API");

        let mut result;

        // For each entry processed by the sequence buffer, pop it out, write the block, and write the block id
        // FORMAT: Write each sequence block to file

        // This writes out the sequence blocks (seqblockcompressed)
        let start = out_buf.seek(SeekFrom::Current(0)).unwrap();
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
                    bincode::encode_into_std_write(&sb, &mut out_buf, bincode_config)
                        .expect("Unable to write to bincode output");
                    // log::info!("Writer wrote block {}", block_id);

                    block_locs.push((block_id, pos));

                    pos = out_buf
                        .seek(SeekFrom::Current(0))
                        .expect("Unable to work with seek API");
                }
            }
        }

        fasta_thread.join().unwrap();

        let end = out_buf.seek(SeekFrom::Current(0)).unwrap();
        log::info!("DEBUG: Wrote {} bytes of sequence blocks", end - start);
        debug_size.push(("Sequence Blocks".to_string(), (end - start) as usize));

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

        log::info!("DEBUG: Wrote {} total blocks", block_locs.len());

        let (num_bits, bitpacked) = bitpack_u32(block_locs_u32);
        bincode::encode_into_std_write(num_bits, &mut out_buf, bincode_config)
            .expect("Unable to write to bincode output");

        let size_loc = out_buf
            .seek(SeekFrom::Current(0))
            .expect("Unable to work with seek API");

        // (size of bitpacked data, total number of block locs)
        let mut size: u64 = 0;
        let bitpacked_len = bitpacked.len() as u64;
        bincode::encode_into_std_write((size, bitpacked_len), &mut out_buf, bincode_config)
            .expect("Unable to write to bincode output");

        for i in bitpacked {
            println!(
                "Writing block at {}",
                out_buf.seek(SeekFrom::Current(0)).unwrap()
            );
            if let Ok(x) = bincode::encode_into_std_write(&i, &mut out_buf, bincode_config) {
                if i.is_packed() && size == 0 {
                    size = x as u64;
                } else if size != 0 && i.is_packed() {
                    assert!(x as u64 == size);
                }
            } else {
                panic!("Unable to write to bincode output");
            }
        }

        let end = out_buf.seek(SeekFrom::Current(0)).unwrap();

        out_buf.seek(SeekFrom::Start(size_loc)).unwrap();
        bincode::encode_into_std_write((size, bitpacked_len), &mut out_buf, bincode_config)
            .expect("Unable to write to bincode output");

        out_buf.seek(SeekFrom::Start(end)).unwrap();

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

        let start = out_buf.seek(SeekFrom::Current(0)).unwrap();

        headers_location = match headers.as_mut().unwrap().write_to_buffer(&mut out_buf) {
            Some(x) => NonZeroU64::new(x),
            None => None,
        };

        let end = out_buf.seek(SeekFrom::Current(0)).unwrap();

        let end_time = Instant::now();

        debug_size.push(("Headers".to_string(), (end - start) as usize));

        log::info!(
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

        debug_size.push(("IDs".to_string(), (end - start) as usize));
        log::info!(
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
        debug_size.push(("Masking".to_string(), (end - start) as usize));
        let end_time = Instant::now();
        log::info!(
            "DEBUG: Wrote {} bytes of masking in {:#?}",
            end - start,
            end_time - start_time
        );

        out_buf.flush().expect("Unable to flush output buffer");

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

// We have to iterate through the file twice...
// TODO:
#[allow(dead_code)]
pub fn write_fastq_sequence<'write, W, R>(
    sb_config: CompressionStreamBufferConfig,
    in_buf: &mut R,
    mut out_fh: &mut W,
) -> (
    Vec<std::sync::Arc<String>>,
    SeqLocs<'write>,
    u64,
    Option<NonZeroU64>,
    Option<NonZeroU64>,
    Option<NonZeroU64>,
)
where
    W: WriteAndSeek + 'write,
    R: Read + Send + Seek + 'write,
{
    let bincode_config = bincode::config::standard().with_fixed_int_encoding();

    let mut sb = CompressionStreamBuffer::from_config(sb_config.clone());

    // Get a handle for the output queue from the sequence compressor
    let oq = sb.get_output_queue();

    // Get a handle to the shutdown flag...
    let shutdown = sb.get_shutdown_flag();

    let mut seq_locs: Option<SeqLocs> = None;
    let mut block_index_pos = None;
    let mut headers = None;
    let mut headers_location = None;
    let mut ids = None;
    let mut ids_location = None;
    let mut ids_string = Vec::new();
    let mut masking = None;
    let mut masking_location = None;

    let fastq_queue: Arc<ArrayQueue<Work>> = std::sync::Arc::new(ArrayQueue::new(8192));

    let fastq_queue_in = Arc::clone(&fastq_queue);
    let fastq_queue_out = Arc::clone(&fastq_queue);

    // Pop to a new thread that pushes FASTQ sequences into the sequence buffer...
    thread::scope(|s| {
        // Sequence thread....
        let fastq_thread = s.spawn(|_| {
            // Convert reader into buffered reader then into the Fastq struct (and iterator)
            let mut in_buf_reader = BufReader::new(in_buf);
            let fastq = Fastq::from_buffer(&mut in_buf_reader);

            let backoff = Backoff::new();

            for x in fastq {
                let mut x = x.unwrap();
                x.scores.take(); // No need to send scores across threads when we aren't using them yet...
                let mut d = Work::FastqPayload(x);
                while let Err(z) = fastq_queue_in.push(d) {
                    d = z;
                    backoff.snooze();
                }
            }

            while fastq_queue_in.push(Work::Shutdown).is_err() {
                backoff.snooze();
            }

            // Return the in_buf
            in_buf_reader.into_inner()
        });

        let mut out_buf = BufWriter::new(&mut out_fh);
        let reader_handle = s.spawn(move |_| {
            sb.initialize();

            // Store the ID of the sequence and the Location (SeqLocs)
            // TODO: Auto adjust based off of size of input FASTA file

            let mut seqlocs = SeqLocs::default();
            let mut headers = StringBlockStore::default().with_block_size(1024 * 1024);
            let mut ids = StringBlockStore::default().with_block_size(512 * 1024);
            let mut ids_string = Vec::new();
            let mut masking = Masking::default();

            // For each Sequence in the fasta file, make it upper case (masking is stored separately)
            // Add the sequence, get the SeqLocs and store them in Location struct
            // And store that in seq_locs Vec...

            let backoff = Backoff::new();

            loop {
                match fastq_queue_out.pop() {
                    Some(Work::FastqPayload(seq)) => {
                        let (seqid, seqheader, mut seq, _) = seq.into_parts();
                        let mut location = SeqLoc::new();
                        let masked = masking.add_masking(&seq.as_ref().unwrap()[..]);
                        if let Some(x) = masked {
                            let x = seqlocs.add_locs(&x);
                            location.masking = Some(x);
                        }
                        let loc = sb.add_sequence(&mut seq.unwrap()[..]).unwrap(); // Destructive, capitalizes everything...
                        let myid = std::sync::Arc::new(seqid.unwrap());
                        ids_string.push(std::sync::Arc::clone(&myid));
                        let idloc = ids.add(&(*myid));
                        if let Some(header) = seqheader {
                            let x = headers.add(header);
                            let x = seqlocs.add_locs(&x);
                            location.headers = Some((x.0, x.1 as u8));
                        }
                        let x = seqlocs.add_locs(&loc);
                        location.ids = Some((x.0, x.1 as u8));
                        location.sequence = Some(seqlocs.add_locs(&loc));
                        seqlocs.add_to_index(location);
                    }
                    Some(Work::FastaPayload(_)) => {
                        panic!("Received Fastq Payload in FASTQ thread.");
                    }
                    Some(Work::Shutdown) => break,
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
            (seqlocs, headers, ids, ids_string, masking)
        });

        // Store the location of the Sequence Blocks...
        // Stored as Vec<(u32, u64)> because the multithreading means it does not have to be in order
        let mut block_locs = Vec::with_capacity(8192);
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
                    // log::info!("Writer wrote block {}", block_id);

                    block_locs.push((block_id, pos));

                    pos = out_buf
                        .seek(SeekFrom::Current(0))
                        .expect("Unable to work with seek API");
                }
            }
        }

        let in_buf = fastq_thread.join().unwrap();
        let j = reader_handle.join().unwrap();

        let end = out_buf.seek(SeekFrom::Current(0)).unwrap();
        log::info!("DEBUG: Wrote {} bytes of sequence blocks", end - start);

        let _start = out_buf.seek(SeekFrom::Current(0)).unwrap();
        // Write FASTQ scores here...
        let fastq_thread = s.spawn(|_| {
            // Convert reader into buffered reader then into the Fastq struct (and iterator)
            let mut in_buf_reader = BufReader::new(in_buf);
            let fastq = Fastq::from_buffer(&mut in_buf_reader);

            let backoff = Backoff::new();

            for x in fastq {
                let mut x = x.unwrap();
                x.sequence.take(); // No need to send seqs across threads when we are done with that...
                let mut d = Work::FastqPayload(x);
                while let Err(z) = fastq_queue_in.push(d) {
                    d = z;
                    backoff.snooze();
                }
            }

            while fastq_queue_in.push(Work::Shutdown).is_err() {
                backoff.snooze();
            }
        });

        let mut sb = CompressionStreamBuffer::from_config(sb_config);
        let _fastq_queue_in = Arc::clone(&fastq_queue);
        let fastq_queue_out = Arc::clone(&fastq_queue);

        let scores_handle = s.spawn(move |_| {
            sb.initialize();

            // Store the ID of the sequence and the Location (SeqLocs)
            // TODO: Auto adjust based off of size of input FASTA file
            let mut seq_locs: Vec<SeqLoc> = Vec::with_capacity(1024);

            // For each Sequence in the fasta file, make it upper case (masking is stored separately)
            // Add the sequence, get the SeqLocs and store them in Location struct
            // And store that in seq_locs Vec...

            let backoff = Backoff::new();

            let mut idx = 0;

            loop {
                match fastq_queue_out.pop() {
                    Some(Work::FastqPayload(seq)) => {
                        idx += 1;
                        let (_, _, _, mut scores) = seq.into_parts();
                        let loc = sb.add_sequence(&mut scores.unwrap()[..]).unwrap();
                        // Destructive, capitalizes everything...
                        // TODO:
                        // seqlocs.index.as_mut().unwrap().scores = Some(loc);
                    }
                    Some(Work::FastaPayload(_)) => {
                        panic!("Received Fastq Payload in FASTQ thread.");
                    }
                    Some(Work::Shutdown) => break,
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
            seq_locs
        });

        // FASTQ sequence blocks, etc...
        // All the following is TODO

        // Store the location of the Sequence Blocks...
        // Stored as Vec<(u32, u64)> because the multithreading means it does not have to be in order
        let mut block_locs = Vec::with_capacity(8192);
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
                    // log::info!("Writer wrote block {}", block_id);

                    block_locs.push((block_id, pos));

                    pos = out_buf
                        .seek(SeekFrom::Current(0))
                        .expect("Unable to work with seek API");
                }
            }
        }

        fastq_thread.join().unwrap();

        let end = out_buf.seek(SeekFrom::Current(0)).unwrap();
        log::info!("DEBUG: Wrote {} bytes of sequence blocks", end - start);

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
        log::info!("DEBUG: Wrote {} bytes of block index", end - start);

        // let j = reader_handle.join().expect("Unable to join thread");
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
        log::info!(
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
        log::info!(
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
        log::info!(
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
