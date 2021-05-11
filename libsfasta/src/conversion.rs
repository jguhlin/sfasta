// Easy conversion functions
use std::fs::{metadata, File};
use std::hash::Hasher;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom};
use std::sync::atomic::Ordering;
use std::thread;

use crossbeam::utils::Backoff;
use twox_hash::XxHash64;

use crate::fasta::*;
use crate::format::Sfasta;
use crate::sequence_buffer::SequenceBuffer;
use crate::structs::WriteAndSeek;
use crate::types::*;
use crate::index::IDIndexer;

// TODO: Add support for metadata here...
// TODO: Will likely need to be the same builder style
// TODO: Will need to generalize this function so it works with FASTA & FASTQ & Masking
pub fn convert_fasta<W>(in_filename: &str, out_buf: &mut W, block_size: u32, threads: u16)
where
    W: WriteAndSeek,
{
    //    let input = generic_open_file(filename);
    //    let input = Box::new(BufReader::with_capacity(512 * 1024, input.2));

    assert!(block_size < u32::MAX, "Block size must be less than u32::MAX (~4Gb)");

    let mut sfasta = Sfasta::default().block_size(block_size).with_sequences(); // This is a FASTA, so no scores

    // Output file
    // let out_file = File::create(output_filename.clone()).expect("Unable to write to file");
    let mut out_fh = BufWriter::with_capacity(1024 * 1024, out_buf);

    // Write the directory, parameters, and metadata structs out...
    bincode::serialize_into(&mut out_fh, &sfasta.directory)
        .expect("Unable to write directory to file");
    bincode::serialize_into(&mut out_fh, &sfasta.parameters)
        .expect("Unable to write Parameters to file");
    bincode::serialize_into(&mut out_fh, &sfasta.metadata)
        .expect("Unable to write Metadata to file");

    let mut sb = SequenceBuffer::default()
        .with_block_size(block_size)
        .with_threads(threads); // Effectively # of compression threads

    let oq = sb.output_queue();
    let shutdown = sb.shutdown_notified();

    let in_filename = in_filename.to_string();

    // Pop to a new thread that pushes FASTA sequences into the sequence buffer...
    let reader_handle = thread::spawn(move || {
        let (_, _, in_buf) = generic_open_file(&in_filename);
        let fasta = Fasta::from_buffer(BufReader::with_capacity(512 * 1024, in_buf));

        let mut seq_locs = Vec::new();
        for i in fasta {
            let loc = sb.add_sequence(&i.seq[..]).unwrap();
            seq_locs.push((i.id, loc));
        }

        // Finalize pushes the last block, which is likely smaller than the complete block size
        match sb.finalize() {
            Ok(()) => (),
            Err(x) => panic!("Unable to finalize sequence buffer, {:#?}", x),
        };

        return seq_locs;
    });

    let mut block_locs = Vec::new();
    let backoff = Backoff::new();
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
                backoff.snooze();
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

    // TODO: Write out the index

    // Write out the sequence locs...
    let seq_locs: Vec<(String, Vec<Loc>)> = reader_handle.join().unwrap();
    
    let mut indexer = crate::index::Index64::default();
    let mut pos = out_fh
        .seek(SeekFrom::Current(0))
        .expect("Unable to work with seek API");

    for s in seq_locs {
        indexer.add(&s.0, pos);
        bincode::serialize_into(&mut out_fh, &s)
            .expect("Unable to write Metadata to file");
        
        pos = out_fh
            .seek(SeekFrom::Current(0))
            .expect("Unable to work with seek API");
    }

    let indexer = indexer.finalize();

    // ID Index
    let id_index_pos = out_fh
        .seek(SeekFrom::Current(0))
        .expect("Unable to work with seek API");

    bincode::serialize_into(&mut out_fh, &indexer)
        .expect("Unable to write directory to file");

    // Block Index
    let block_index_pos = out_fh
        .seek(SeekFrom::Current(0))
        .expect("Unable to work with seek API");

    bincode::serialize_into(&mut out_fh, &block_locs)
        .expect("Unable to write directory to file");

    // TODO: Scores Block Index
    
    // TODO: Masking Block index

    // Go to the beginning, and write the location of the index

    sfasta.directory.index_loc = id_index_pos;
    sfasta.directory.block_index_loc = block_index_pos;
    out_fh.seek(SeekFrom::Start(0)).expect("Unable to rewind to start of the file");
    // Here we re-write the directory information at the start of the file, allowing for
    // easy hops to important areas while keeping everything in a single file
    bincode::serialize_into(&mut out_fh, &sfasta.directory)
        .expect("Unable to write directory to file");

    //panic!();
}

#[inline]
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
    use crate::sequence_block::{SequenceBlock, SequenceBlockCompressed};
    use std::io::Cursor;

    #[test]
    pub fn test_create_sfasta() {
        let mut out_buf = Cursor::new(Vec::new());

        convert_fasta("test_data/test_convert.fasta", &mut out_buf, 8 * 1024, 6);

        match out_buf.seek(SeekFrom::Start(0)) {
            Err(x) => panic!("Unable to seek to start of file, {:#?}", x),
            Ok(_) => (),
        };

        let d: Directory = bincode::deserialize_from(&mut out_buf).unwrap();
        let p: Parameters = bincode::deserialize_from(&mut out_buf).unwrap();
        let m: Metadata = bincode::deserialize_from(&mut out_buf).unwrap();

        let b: SequenceBlockCompressed = bincode::deserialize_from(&mut out_buf).unwrap();
        let b = b.decompress();

        assert!(b.len() == 55);
    }
}
