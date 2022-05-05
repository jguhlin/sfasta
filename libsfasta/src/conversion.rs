// Easy, high-performance conversion functions
use crossbeam::thread;
use std::borrow::Cow;
use std::convert::TryFrom;
use std::fs::{metadata, File};
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::num::NonZeroU64;
use std::ops::DerefMut;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use rayon::prelude::*;

use crate::compression_stream_buffer::CompressionStreamBuffer;
use crate::fasta::*;
use crate::format::DirectoryOnDisk;
use crate::format::{DualIndex, Sfasta, IDX_CHUNK_SIZE, SEQLOCS_CHUNK_SIZE};
use crate::index::{IDIndexer, StoredIndexPlan};
use crate::structs::WriteAndSeek;
use crate::types::*;
use crate::utils::*;
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
    dict: Option<Vec<u8>>,
}

impl Default for Converter {
    fn default() -> Self {
        Converter {
            threads: 8,
            block_size: 8 * 1024 * 1024,    // 8Mb
            index_chunk_size: 128 * 1024,   // 128k
            seqlocs_chunk_size: 128 * 1024, // 128k
            index_compression_type: CompressionType::ZSTD,
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
        unimplemented!();

        // self.chunk_size =

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
    pub fn convert_fasta<'convert, W, R>(self, in_buf: R, mut out_buf: W) -> W
    where
        W: WriteAndSeek + 'convert + std::fmt::Debug,
        R: Read + Send + 'convert,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        let mut sfasta = Sfasta::default().block_size(self.block_size as u32);

        // Store masks as series of 0s and 1s... Vec<bool>
        // TODO
        if self.masking {
            sfasta = sfasta.with_masking();
        }

        sfasta.parameters.compression_type = self.compression_type;
        sfasta.parameters.index_compression_type = self.index_compression_type;
        sfasta.parameters.index_chunk_size = self.index_chunk_size as u32;
        sfasta.parameters.seqlocs_chunk_size = self.seqlocs_chunk_size as u32;

        // Set dummy values for the directory
        sfasta.directory.dummy();

        // Store the location of the directory so we can update it later...
        // It's right after the SFASTA and version identifier...
        let directory_location = self.write_headers(&mut out_buf, &sfasta);

        // Put everything in Arcs and Mutexes to allow for multithreading
        // let in_buf = Arc::new(Mutex::new(in_buf));
        // let out_buf = Arc::new(Mutex::new(out_buf));

        // Write sequences
        let sb = CompressionStreamBuffer::default()
            .with_block_size(self.block_size as u32)
            .with_compression_type(self.compression_type)
            .with_threads(self.threads as u16); // Effectively # of compression threads

        // This returns in_buf, but is not currently used (hence the _)
        // Input filehandle goes in, output goes out.
        // Function returns:
        // Vec<(String, Location)>
        // block_index_pos
        // in_buffer
        // out_buffer
        let (seq_locs, block_index_pos, mut _in_buf, mut out_buf) =
            write_fasta_sequence(sb, in_buf, out_buf);
        //write_fasta_sequence(sb, Arc::clone(&in_buf), Arc::clone(&out_buf));

        // TODO: Here is where we would write out the scores...
        // ... but this fn is only for FASTA right now...

        // TODO: Here is where we would write out the masking...

        // TODO: Here is where we would write out the Seqinfo stream (if it's decided to do it)

        // TODO: Support for Index32 (and even smaller! What if only 1 or 2 sequences?)
        let mut indexer = crate::index::Index64Builder::with_capacity(seq_locs.len()).with_ids();
        let mut indexer_final = None;

        // Write out the sequence locs...
        //         let seq_locs: Vec<(Cow<str>, Location)> = reader_handle.join().unwrap();

        let seqlocs_loc = out_buf
            .seek(SeekFrom::Current(0))
            .expect("Unable to work with seek API");

        sfasta.directory.seqlocs_loc = NonZeroU64::new(seqlocs_loc);

        let mut out_fh = BufWriter::with_capacity(8 * 1024 * 1024, out_buf);

        let mut seqlocs_blocks_locs: Vec<u64> =
            Vec::with_capacity((seq_locs.len() / SEQLOCS_CHUNK_SIZE) + 1);

        let seq_locs = Arc::new(seq_locs);

        // The index points to the location of the Location structs.
        // Location blocks are chunked into SEQLOCS_CHUNK_SIZE
        // So index.get(id) --> integer position of the location struct
        // Which will be in integer position / SEQLOCS_CHUNK_SIZE chunk offset
        // This will then point to the different location blocks where the sequence is...
        //
        // TODO: Optional index can probably be handled better...

        let to_index = Arc::clone(&seq_locs);

        thread::scope(|s| {
            let index_handle = Some(s.spawn(|_| {
                println!("{:#?}", to_index.len());

                for (i, (id, _)) in to_index.iter().enumerate() {
                    indexer.add(id, i as u32).expect("Unable to add to index");
                }

                indexer
            }));

            // FORMAT: Write sequence location blocks
            for s in seq_locs
                .iter()
                .collect::<Vec<&(String, Location)>>()
                .chunks(SEQLOCS_CHUNK_SIZE)
            {
                seqlocs_blocks_locs.push(
                    out_fh
                        .seek(SeekFrom::Current(0))
                        .expect("Unable to work with seek API"),
                );

                let locs: Vec<_> = s.iter().map(|(_, l)| l).collect();

                let mut compressor =
                    zstd::stream::Encoder::new(Vec::with_capacity(8 * 1024 * 1024), -3).unwrap();

                bincode::encode_into_std_write(&locs, &mut compressor, bincode_config)
                    .expect("Unable to bincode locs into compressor");
                let compressed = compressor.finish().unwrap();

                bincode::encode_into_std_write(compressed, &mut out_fh, bincode_config)
                    .expect("Unable to write Sequence Blocks to file");
            }

            if let Some(indexerh) = index_handle {
                indexer_final = Some(indexerh.join().unwrap());
                println!("Got indexer_final");
            } else {
                // TODO: This should never be called... it's here to make the compiler happy
                // indexer = crate::index::Index64::with_capacity(1);
                unreachable!();
            }
        })
        .expect("Error");

        println!("Finalizing...");

        let mut indexer = indexer_final.unwrap().finalize();
        println!("Finalizing...Done");

        // ID Index
        let id_index_pos = out_fh
            .seek(SeekFrom::Current(0))
            .expect("Unable to work with seek API");

        let ids = indexer.ids.take().unwrap();

        let (hashes, bitpacked, hash_type, min_size) = indexer.into_parts();

        let (mut plan, parts) =
            StoredIndexPlan::plan_from_parts(&hashes, &bitpacked, hash_type, min_size);

        // FORMAT: Index Plan
        let plan_loc = out_fh
            .seek(SeekFrom::Current(0))
            .expect("Unable to work with seek API");

        bincode::encode_into_std_write(&plan, &mut out_fh, bincode_config)
            .expect("Unable to bincode Index Plan");

        for (n, part) in parts.into_iter().enumerate() {
            plan.index[n].1 = out_fh
                .seek(SeekFrom::Current(0))
                .expect("Unable to work with seek API");

            let mut compressor =
                zstd::stream::Encoder::new(Vec::with_capacity(16 * 1024 * 1024), -9).unwrap();

            let part = part.to_vec();

            bincode::encode_into_std_write(part, &mut compressor, bincode_config)
                .expect("Unable to bincode index to compressor");
            let compressed = compressor.finish().unwrap();

            bincode::encode_into_std_write(compressed, &mut out_fh, bincode_config)
                .expect("Unable to bincode compressed hashes");

            let new_pos = out_fh
                .seek(SeekFrom::Current(0))
                .expect("Unable to work with seek API");
        }

        let bitpacked_loc = out_fh
            .seek(SeekFrom::Current(0))
            .expect("Unable to work with seek API");

        // Re-write Plan since it now has indexes...
        out_fh
            .seek(SeekFrom::Start(plan_loc))
            .expect("Unable to seek to plan loc");

        bincode::encode_into_std_write(plan, &mut out_fh, bincode_config)
            .expect("Unable to bincode plan");

        // IMPORTANT: Jump back to where we were!

        out_fh
            .seek(SeekFrom::Start(bitpacked_loc))
            .expect("Unable to work with seek API");

        let mut bitpacked_dual_index = DualIndex::new(bitpacked_loc);

        println!("Len: {}", bitpacked.len());

        for bp in bitpacked {
            let pos = out_fh
                .seek(SeekFrom::Current(0))
                .expect("Unable to work with seek API");
            bitpacked_dual_index.locs.push(pos);

            bincode::encode_into_std_write(bp, &mut out_fh, bincode_config)
                .expect("Unable to bincode index hash type");
        }

        // TODO: Save this location to the directory...
        bitpacked_dual_index.write_to_buffer(&mut out_fh);

        let index_seqlocs_blocks_locs = out_fh
            .seek(SeekFrom::Current(0))
            .expect("Unable to work with seek API");

        let ids_loc = out_fh
            .seek(SeekFrom::Current(0))
            .expect("Unable to work with seek API");

        let mut ids_blocks_locs: Vec<u64> = Vec::with_capacity(ids.len() / IDX_CHUNK_SIZE);

        let compressed_blocks = ids
            .chunks(IDX_CHUNK_SIZE)
            .collect::<Vec<&[String]>>()
            .par_iter()
            .map(|&chunk| {
                // let output: Vec<u8> = Vec::with_capacity(4 * 1024 * 1024);
                // let mut compressor = lz4_flex::frame::FrameEncoder::new(output);
                let mut compressor =
                    zstd::stream::Encoder::new(Vec::with_capacity(4 * 1024 * 1024), 11).unwrap();

                compressor.write(
                    &bincode::encode_to_vec(Vec::from(chunk), bincode_config)
                        .expect("Unable to write chunk to compressor"),
                );

                compressor.finish().expect("Unable to compress ID stream")
            })
            .collect::<Vec<Vec<u8>>>();

        for block in compressed_blocks.into_iter() {
            let pos = out_fh
                .seek(SeekFrom::Current(0))
                .expect("Unable to work with seek API");

            ids_blocks_locs.push(pos);

            bincode::encode_into_std_write(block, &mut out_fh, bincode_config)
                .expect("Unable to write directory to file");
        }

        // ID Block Locs

        let id_blocks_index_loc = out_fh
            .seek(SeekFrom::Current(0))
            .expect("Unable to work with seek API");

        let ids_blocks_locs: Vec<u64> = ids_blocks_locs.into_iter().map(|x| x - ids_loc).collect();

        // TODO: Handle this pre-emptively with a flag or something...
        assert!(ids_blocks_locs.iter().max().unwrap() <= &(u32::MAX as u64), "Edge case, too many IDs... please e-mail Joseph and I can fix this in the next release");
        let ids_blocks_locs: Vec<u32> = ids_blocks_locs
            .into_iter()
            .map(|x| u32::try_from(x).unwrap())
            .collect();

        let bitpacked = bitpack_u32(&ids_blocks_locs);

        bincode::encode_into_std_write(bitpacked, &mut out_fh, bincode_config)
            .expect("Unable to write directory to file");

        // SeqLoc Blocks Locs
        let seqloc_blocks_index_loc = out_fh
            .seek(SeekFrom::Current(0))
            .expect("Unable to work with seek API");

        let seqlocs_blocks_locs: Vec<u64> = seqlocs_blocks_locs
            .into_iter()
            .map(|x| x - seqlocs_loc)
            .collect();

        let mut compressor =
            zstd::stream::Encoder::new(Vec::with_capacity(4 * 1024 * 1024), 3).unwrap();

        compressor
            .write(&bincode::encode_to_vec(seqlocs_blocks_locs, bincode_config).unwrap())
            .expect("Unable to write directory to file");

        let compressed = compressor.finish().expect("Unable to compress ID stream");

        bincode::encode_into_std_write(compressed, &mut out_fh, bincode_config)
            .expect("Unable to write directory to file");

        sfasta.directory.seqloc_blocks_index_loc = NonZeroU64::new(seqloc_blocks_index_loc);
        sfasta.directory.id_blocks_index_loc = NonZeroU64::new(id_blocks_index_loc);
        sfasta.directory.index_loc = NonZeroU64::new(id_index_pos);
        sfasta.directory.ids_loc = NonZeroU64::new(ids_loc);
        sfasta.directory.index_plan_loc = NonZeroU64::new(plan_loc);
        sfasta.directory.index_bitpacked_loc = NonZeroU64::new(bitpacked_loc);

        // TODO: Scores Block Index

        // TODO: Masking Block
        // Use bitvec implementation

        // Go to the beginning, and write the location of the index

        sfasta.directory.block_index_loc = NonZeroU64::new(block_index_pos);

        out_fh
            .seek(SeekFrom::Start(directory_location))
            .expect("Unable to rewind to start of the file");

        // Here we re-write the directory information at the start of the file, allowing for
        // easy jumps to important areas while keeping everything in a single file
        let dir: DirectoryOnDisk = sfasta.directory.clone().into();
        bincode::encode_into_std_write(dir, &mut out_fh, bincode_config)
            .expect("Unable to write directory to file");

        out_fh.into_inner().unwrap()
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

/// Input filehandle goes in, output goes out.
/// Function returns:
/// Vec<(String, Location)>
/// block_index_pos
/// in_buffer
/// out_buffer

pub fn write_fasta_sequence<'write, W, R>(
    mut sb: CompressionStreamBuffer,
    in_buf: R,
    mut out_fh: W,
) -> (Vec<(String, Location)>, u64, R, W)
where
    W: WriteAndSeek + 'write,
    R: Read + Send + 'write,
{
    // let in_buf = in_buf.get_mut().unwrap();
    // let in_buf = BufReader::new(in_buf);

    // TODO: I don't believe this is functional right now...
    // if self.dict.is_some() {
    // sb = sb.with_dict(self.dict.unwrap());
    // }

    let bincode_config = bincode::config::standard().with_fixed_int_encoding();

    // Get a handle for the output queue from the sequence compressor
    let oq = sb.output_queue();

    // Get a handle to the shutdown flag...
    let shutdown = sb.shutdown_notified();

    let mut seq_locs: Option<Vec<(String, Location)>> = None;
    let mut block_index_pos = None;

    let mut return_in_buf: Option<R> = None;

    // Pop to a new thread that pushes FASTA sequences into the sequence buffer...
    thread::scope(|s| {
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
                let mut location = Location::new();
                location.sequence = Some(loc);
                seq_locs.push((i.id.to_string(), location));
            }

            // Finalize pushes the last block, which is likely smaller than the complete block size
            match sb.finalize() {
                Ok(()) => (),
                Err(x) => panic!("Unable to finalize sequence buffer, {:#?}", x),
            };

            // Return seq_locs to the main thread
            let in_buf = in_buf_reader.into_inner();
            (Some(seq_locs), Some(in_buf))
        });

        //        let mut out_fh_lock = out_fh.lock().unwrap();
        //        let mut out_fh = out_fh_lock.deref_mut();

        // Store the location of the Sequence Blocks...
        // Stored as Vec<(u32, u64)> because the multithreading means it does not have to be in order
        let mut block_locs = Vec::with_capacity(512 * 1024);
        let mut pos = out_fh
            .seek(SeekFrom::Current(0))
            .expect("Unable to work with seek API");

        let mut result;

        // For each entry processed by the sequence buffer, pop it out, write the block, and write the block id
        // FORMAT: Write each sequence block to file

        // This writes out the sequence blocks (seqblockcompressed)
        loop {
            result = oq.pop();

            match result {
                None => {
                    if oq.is_empty() && shutdown.load(Ordering::Relaxed) {
                        break;
                    }
                }
                Some((block_id, sb)) => {
                    bincode::encode_into_std_write(sb, &mut out_fh, bincode_config)
                        .expect("Unable to write to bincode output");

                    block_locs.push((block_id, pos));

                    pos = out_fh
                        .seek(SeekFrom::Current(0))
                        .expect("Unable to work with seek API");
                }
            }
        }

        // Block Index
        block_index_pos = Some(
            out_fh
                .seek(SeekFrom::Current(0))
                .expect("Unable to work with seek API"),
        );

        // block_locs.sort_by(|a, b| a.0.cmp(&b.0));

        // let block_locs: Vec<u64> = block_locs.iter().map(|x| x.1).collect();

        //let mut seqblocks_locs = DualIndex::new(block_locs[0]);
        //seqblocks_locs.block_locs = block_locs;
        //seqblocks_locs.write_to_buffer(&mut out_fh);

        (seq_locs, return_in_buf) = reader_handle.join().unwrap();
    })
    .expect("Error");

    (
        seq_locs.expect("Error"),
        block_index_pos.expect("Error"),
        return_in_buf.unwrap(),
        out_fh,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::Directory;
    use crate::metadata::Metadata;
    use crate::parameters::Parameters;
    use crate::sequence_block::SequenceBlockCompressed;
    use std::io::Cursor;

    #[test]
    pub fn test_create_sfasta() {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        let mut out_buf = Cursor::new(Vec::new());

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

        let dir: DirectoryOnDisk = directory.into();
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
