use std::convert::From;
use std::convert::TryFrom;
use std::fs::{metadata, File};
use std::io::prelude::*;
use std::io::{BufReader, Read};
use std::io::{BufWriter, Seek, SeekFrom};
use std::path::Path;
use std::time::Instant;

use hashbrown::HashMap;
use serde::{Deserialize, Serialize};

use crate::structs::{
    EntryCompressedBlock, EntryCompressedHeader, Header, ReadAndSeek, WriteAndSeek,
};
use crate::utils::{check_extension, get_index_filename};

pub struct Sequences {
    reader: Box<dyn Read + Send>,
}

#[derive(PartialEq, Serialize, Deserialize, Clone, Debug)]
pub struct Sequence {
    pub seq: Vec<u8>,
    pub id: String,
    pub location: usize,
    pub end: usize,
}

// TODO: This is the right place to do this, but I feel it's happening somewhere
// else and wasting CPU cycles...
impl Sequence {
    pub fn make_uppercase(&mut self) {
        self.seq.make_ascii_uppercase();
    }
}

/*fn open_file(filename: String) -> Box<dyn Read + Send> {
    let file = match File::open(&filename) {
        Err(why) => panic!("Couldn't open {}: {}", filename, why.to_string()),
        Ok(file) => file,
    };

    let file = BufReader::with_capacity(64 * 1024 * 1024, file);

    let fasta: Box<dyn Read + Send> = if filename.ends_with("gz") {
        Box::new(flate2::read::GzDecoder::new(file))
    } else if filename.ends_with("snappy") || filename.ends_with("sz") {
        Box::new(snap::read::FrameDecoder::new(file))
    } else {
        Box::new(file)
    };

    let reader = BufReader::with_capacity(32 * 1024 * 1024, fasta);
    Box::new(reader)
} */

impl Iterator for Sequences {
    type Item = Sequence;

    fn next(&mut self) -> Option<Sequence> {
        let seq: Sequence = match bincode::deserialize_from(&mut self.reader) {
            Ok(x) => x,
            Err(_) => {
                println!("SeqStop");
                return None;
            }
        };
        Some(seq)
    }
}
/*
impl Sequences {
    pub fn new(filename: String) -> Sequences {
        Sequences {
            reader: open_file(filename),
        }
    }
} */

/// Opens an SFASTA file and an index and returns a Box<dyn Read>,
/// HashMap<String, usize> type
pub fn open_file(
    filename: &str,
) -> (
    Box<dyn ReadAndSeek + Send>,
    Option<(Vec<String>, Vec<u64>, Vec<(String, u64)>, Vec<u64>)>,
) {
    let filename = check_extension(filename);

    let file = match File::open(Path::new(&filename)) {
        Err(_) => panic!("Couldn't open {}", filename),
        Ok(file) => file,
    };

    let reader = BufReader::with_capacity(512 * 1024, file);

    (Box::new(reader), load_index(&filename))
}

/// Get all IDs from an SFASTA file
/// Really a debugging function...
pub fn get_headers_from_sfasta(filename: String) -> Vec<String> {
    let file = match File::open(&filename) {
        Err(why) => panic!("Couldn't open {}: {}", filename, why.to_string()),
        Ok(file) => file,
    };

    let mut reader = BufReader::with_capacity(512 * 1024, file);

    let header: Header = match bincode::deserialize_from(&mut reader) {
        Ok(x) => x,
        Err(_) => panic!("Header missing or malformed in SFASTA file"),
    };

    let mut ids: Vec<String> = Vec::with_capacity(2048);

    while let Ok(entry) = bincode::deserialize_from::<_, EntryCompressedHeader>(&mut reader) {
        ids.push(entry.id);
        for _i in 0..entry.block_count as usize {
            let _x: EntryCompressedBlock = bincode::deserialize_from(&mut reader).unwrap();
        }
    }

    ids
}
/*
/// Indexes an SFASTA file
pub fn index(filename: &str) {
    // TODO: Run a sanity check on the file first... Make sure it's valid
    // sfasta

    let filesize = metadata(&filename).expect("Unable to open file").len();
    let starting_size = std::cmp::max((filesize / 500) as usize, 1024);

    //    let mut idx: HashMap<String, u64, RandomXxHashBuilder64> =
    // Default::default();    idx.reserve(starting_size);

    let mut ids = Vec::with_capacity(starting_size);
    let mut locations = Vec::with_capacity(starting_size);

    let mut block_ids = Vec::with_capacity(starting_size);
    let mut block_locations = Vec::with_capacity(starting_size);

    let fh = match File::open(&filename) {
        Err(why) => panic!("Couldn't open {}: {}", filename, why.to_string()),
        Ok(file) => file,
    };

    let mut fh = BufReader::with_capacity(512 * 1024, fh);
    let mut pos = fh
        .seek(SeekFrom::Current(0))
        .expect("Unable to work with seek API");
    let mut i = 0;
    let mut now = Instant::now();
    //    let mut bump = Bump::new();
    //    let mut maxalloc: usize = 0;

    let header: Header = match bincode::deserialize_from(&mut fh) {
        Ok(x) => x,
        Err(_) => panic!("Header missing or malformed in SFASTA file"),
    };

    while let Ok(entry) = bincode::deserialize_from::<_, EntryCompressedHeader>(&mut fh) {
        i += 1;
        if i % 100_000 == 0 {
            println!("100k at {} ms.", now.elapsed().as_millis()); //Maxalloc {} bytes", now.elapsed().as_secs(), maxalloc);
            println!(
                "{}/{} {}",
                pos,
                filesize,
                (pos as f32 / filesize as f32) as f32
            );
            now = Instant::now();
        }

        ids.push(entry.id.clone());
        locations.push(pos);

        pos = fh
            .seek(SeekFrom::Current(0))
            .expect("Unable to work with seek API");

        for i in 0..entry.block_count as usize {
            pos = fh
                .seek(SeekFrom::Current(0))
                .expect("Unable to work with seek API");

            let entry: EntryCompressedBlock = match bincode::deserialize_from(&mut fh) {
                Ok(x) => x,
                Err(_) => panic!("Error decoding compressed block"),
            };

            block_ids.push((entry.id.clone(), i as u64));
            block_locations.push(pos as u64);
        }

        pos = fh
            .seek(SeekFrom::Current(0))
            .expect("Unable to work with seek API");
    }

    create_index(filename, ids, locations, block_ids, block_locations)
} */

pub fn load_index(filename: &str) -> Option<(Vec<String>, Vec<u64>, Vec<(String, u64)>, Vec<u64>)> {
    let idx_filename = get_index_filename(filename);

    if !Path::new(&idx_filename).exists() {
        println!("IdxFile does not exist! {} {}", filename, idx_filename);
        return None;
    }

    let (_, _, mut idxfh) = generic_open_file(&idx_filename);
    // let idx: HashMap<String, u64>;
    // let idx: HashMap<String, u64>;
    // idx = bincode::deserialize_from(&mut idxfh).expect("Unable to open Index
    // file");
    let _length: u64 =
        bincode::deserialize_from(&mut idxfh).expect("Unable to read length of index");
    let keys: Vec<String> = bincode::deserialize_from(&mut idxfh).expect("Unable to read idx keys");
    let vals: Vec<u64> = bincode::deserialize_from(&mut idxfh).expect("Unable to read idx values");

    let block_keys: Vec<(String, u64)> =
        bincode::deserialize_from(&mut idxfh).expect("Unable to read idx keys");
    let block_vals: Vec<u64> =
        bincode::deserialize_from(&mut idxfh).expect("Unable to read idx values");

    /*    let mut idxcache = IDXCACHE.get().unwrap().write().unwrap();
    idxcache.insert(
        idx_filename.clone(),
        (
            keys.clone(),
            vals.clone(),
            block_keys.clone(),
            block_vals.clone(),
        ),
    ); */

    Some((keys, vals, block_keys, block_vals))
}

pub fn create_index<W>(
    out_buf: W,
    ids: Vec<String>,
    locations: Vec<u64>,
    block_ids: Vec<(String, u64)>,
    block_locations: Vec<u64>,
) where
    W: WriteAndSeek,
{
    let mut out_buf = BufWriter::with_capacity(1024 * 1024, out_buf);
    let idx: HashMap<String, u64> = ids.into_iter().zip(locations).collect();

    let mut sorted: Vec<_> = idx.into_iter().collect();
    sorted.sort_by(|x, y| x.0.cmp(&y.0));
    let keys: Vec<_> = sorted.iter().map(|x| x.0.clone()).collect();
    let vals: Vec<_> = sorted.iter().map(|x| x.1.clone()).collect();

    // bincode::serialize_into(&mut out_fh, &idx).expect("Unable to write index");

    bincode::serialize_into(&mut out_buf, &(keys.len() as u64)).expect("Unable to write index");
    bincode::serialize_into(&mut out_buf, &keys).expect("Unable to write index");
    bincode::serialize_into(&mut out_buf, &vals).expect("Unable to write index");

    let idx: HashMap<(String, u64), u64> = block_ids.into_iter().zip(block_locations).collect();

    let mut sorted: Vec<_> = idx.into_iter().collect();
    sorted.sort_by(|x, y| x.0.cmp(&y.0));
    let keys: Vec<_> = sorted.iter().map(|x| x.0.clone()).collect();
    let vals: Vec<_> = sorted.iter().map(|x| x.1.clone()).collect();
    bincode::serialize_into(&mut out_buf, &keys).expect("Unable to write index");
    bincode::serialize_into(&mut out_buf, &vals).expect("Unable to write index");
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
}
