extern crate serde;

use ahash::AHasher;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use twox_hash::{XxHash32, XxHash64, Xxh3Hash64};

use std::hash::Hasher;
use std::slice::Chunks;

use crate::utils::*;

#[non_exhaustive]
enum IndexTypes {
    Index32,
    Index64,
}

// TODO: Implement small hasher (HashMap, HashBrown) for very small datasets
// TODO: Implement 16-bit hasher

// Index has morphed into something else now...
// So Index has 3 forms
// Trait supports most of it, but not all
// Index (In-Memory) --> Fastest, but can take awhile to decompress
// Index (as parts, for serialization) --> Not for actual usage...
// Index (on-disk) --> Returns the part of the index that contains the header, then has fn's to decompress
//                 that part...
// Index must convert between all 3
// And handle edge cases (only a few sequences, etc...)

pub trait IDIndexer {
    fn add(&mut self, id: &str, loc: u32) -> Result<(), &'static str>;

    // We return a vector of possible matches
    // We deal with collisions by not-dealing with collisions
    // It's fast enough to take the list of candidates, and query them directly...
    /// Return a vector of possible matches
    /// The match is the usize index location of the appropriate fields.
    /// So the ID will be index.ids[result]
    /// and the location will be index.locs[result]

    fn find(&self, id: &str) -> Option<Vec<usize>>;

    // Finalize (no more additions)
    // This function should handle the sorting (binary search doesn't work without it)
    fn finalize(self) -> Self;

    fn len(&self) -> u64;
    fn is_empty(&self) -> bool;

    fn with_capacity(capacity: usize) -> Self;
    fn new() -> Self;

    fn ids_chunks(&self, chunk_size: usize) -> Chunks<'_, std::string::String>;

    fn set_ids(&mut self, ids: Vec<String>);
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Copy)]
pub enum Hashes {
    Ahash,    // ahash // On fastq file was...  102.68 secs
    XxHash64, // On fastq file was... 96.18
    Xxh3Hash64, // This is not a stable hash right now. Here for future-proofing a bit...
              // On fastq file was... 91.83
}

/// ```
///  // Generate an index and search on it
///  use libsfasta::prelude::*;
///  let mut i64 = Index64::with_capacity(64);
///  for n in 0..512 {
///      let id = format!("test{}", n);
///      i64.add(&id, n).expect("Unable to add to index");
///  }
///  i64.add("TestA", 42);
///  i64.add("TestZ", 84);
///  let mut i64 = i64.finalize(); // Finalize the index
///  let j = i64.find("TestA");
///  let j = j.unwrap()[0];
///  println!("Index location: {} id: {} Location: {}", j, i64.ids.as_ref().unwrap()[j], i64.locs[64]);
///
///  
/// ```
#[derive(Serialize, Deserialize)]
pub struct Index64 {
    hashes: Vec<u64>,
    pub locs: Vec<u32>,
    hash: Hashes,

    #[serde(skip)]
    pub ids: Option<Vec<String>>,
}

impl Default for Index64 {
    fn default() -> Index64 {
        Index64 {
            hashes: Vec::new(),
            locs: Vec::new(),
            hash: Hashes::XxHash64,
            ids: Some(Vec::new()),
        }
    }
}

impl Index64 {
    #[inline]
    fn get_hash(&self, id: &str) -> u64 {
        // TODO: Pretty sure this code could be simplified with dyn Hasher trait...
        // Not sure if a Box<> overhead would be worth it though...

        let id = id.to_lowercase();

        if self.hash == Hashes::Ahash {
            let mut hasher = AHasher::new_with_keys(42, 1010);
            hasher.write(id.as_bytes());
            hasher.finish()
        } else if self.hash == Hashes::Xxh3Hash64 {
            let mut hasher = Xxh3Hash64::with_seed(42);
            hasher.write(id.as_bytes());
            hasher.finish()
        } else {
            let mut hasher = XxHash64::with_seed(42);
            hasher.write(id.as_bytes());
            hasher.finish()
        }
    }

    pub fn reserve(&mut self, capacity: usize) {
        self.hashes.reserve(capacity);
        self.locs.reserve(capacity);
        self.ids.as_mut().unwrap().reserve(capacity);
    }

    pub fn into_parts(self) -> (Vec<u64>, Vec<Bitpacked>, Hashes, u32) {
        let hashes = self.hashes;
        let mut locs = self.locs;
        let min_size = locs.iter().min().unwrap().clone();

        locs = locs
            .into_iter()
            .map(|x| x.saturating_sub(min_size))
            .collect();

        let bitpacked = bitpack_u32(&locs);

        (hashes, bitpacked, self.hash, min_size)
    }

    pub fn from_parts(hashes: Vec<u64>, locs: Vec<u32>, hash: Hashes) -> Index64 {
        Index64 {
            hashes,
            locs,
            hash,
            ids: None,
        }
    }
}

impl IDIndexer for Index64 {
    fn ids_chunks(&self, chunk_size: usize) -> Chunks<'_, std::string::String> {
        self.ids.as_ref().unwrap().chunks(chunk_size)
    }

    fn with_capacity(capacity: usize) -> Self {
        Index64 {
            hashes: Vec::with_capacity(capacity),
            locs: Vec::with_capacity(capacity),
            hash: Hashes::XxHash64,
            ids: Some(Vec::with_capacity(capacity)),
        }
    }

    fn new() -> Self {
        Index64 {
            hashes: Vec::new(),
            locs: Vec::new(),
            hash: Hashes::XxHash64,
            ids: Some(Vec::new()),
        }
    }

    fn add(&mut self, id: &str, loc: u32) -> Result<(), &'static str> {
        let hash = self.get_hash(id);

        self.hashes.push(hash);
        self.locs.push(loc);
        self.ids.as_mut().unwrap().push(id.to_string());

        Ok(())
    }

    // TODO: Dedupe this code with above...
    fn find(&self, id: &str) -> Option<Vec<usize>> {
        let hash = self.get_hash(id);

        let found = match self.hashes.binary_search(&hash) {
            Ok(x) => x,
            Err(_) => return None,
        };

        let mut locs = Vec::new();

        if self.hashes.len() == 1 {
            locs.push(found);
            return Some(locs);
        } else if found == 0 {
            if self.hashes[found + 1] != hash {
                locs.push(found);
                return Some(locs);
            }
        } else if found == self.hashes.len() - 1 {
            if self.hashes[found - 1] != hash {
                locs.push(found);
                return Some(locs);
            }
        } else if self.hashes[found - 1] != hash && self.hashes[found + 1] != hash {
            locs.push(found);
            return Some(locs);
        }

        let mut start = found;
        let mut end = found;

        while self.hashes[start] == hash {
            // Prevent infinte loops
            if start == 0 {
                break;
            }
            start = start.saturating_sub(1);
        }
        start = start.saturating_add(1);

        let len = self.locs.len();

        while end < len && self.hashes[end] == hash {
            end = end.saturating_add(1);
        }

        end = end.saturating_sub(1);

        Some((start..=end).collect())
    }

    fn finalize(mut self) -> Self {
        // TODO: More memory efficient way...
        // But this is a one-time cost so it's hard to justify spending much time or pulling in other crates...

        //let hashes: Vec<u64> = self.ids.as_ref().unwrap().par_iter().map(|x| self.get_hash(x)).collect();

        let ids = self.ids.take().unwrap();

        let mut tuples: Vec<(u64, u32, String)> = izip!(self.hashes, self.locs, ids).collect();

        if tuples.len() >= 256 * 1024 {
            tuples.par_sort_unstable_by(|a, b| a.0.cmp(&b.0));
        } else {
            tuples.sort_by(|a, b| a.0.cmp(&b.0));
        }

        let mut hashes: Vec<u64> = Vec::with_capacity(tuples.len());
        let mut locs: Vec<u32> = Vec::with_capacity(tuples.len());
        let mut ids: Vec<String> = Vec::with_capacity(tuples.len());

        for (hash, loc, id) in tuples.drain(..) {
            hashes.push(hash);
            locs.push(loc);
            ids.push(id);
        }

        // let hashes = tuples.iter().map(|(i, _, _)| *i).collect::<Vec<u64>>();
        // let locs = tuples.iter().map(|(_, o, _)| *o).collect::<Vec<u32>>();

        // .into_iter here so we don't borrow it, and we can just move the Strings rather than clone them
        /*        let ids = ids.into_boxed_slice();
        let ids = tuples
            .into_iter()
            .map(|(_, _, x)| ids[x])
            .collect::<Vec<String>>(); */

        Index64 {
            hashes,
            locs,
            ids: Some(ids),
            hash: self.hash,
        }
    }

    fn len(&self) -> u64 {
        self.locs.len() as u64
    }

    fn is_empty(&self) -> bool {
        self.locs.len() == 0
    }

    fn set_ids(&mut self, ids: Vec<String>) {
        assert!(ids.len() == self.locs.len());
        self.ids = Some(ids);
    }
}

const MINIMUM_CHUNK_SIZE: u32 = 4 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
pub struct StoredIndexPlan {
    pub parts: u16,
    pub index: Vec<(u64, u64)>,
    pub min_size: u32,
    pub hash_type: Hashes,
    pub chunk_size: u32,
    pub index_len: u32,

    #[serde(skip)]
    index64: Option<Index64>,
}

// TODO: If there are multiple IDs matching there could be muultiple blocks
impl StoredIndexPlan {
    // First output is the block, second is the offset (remainder of the divsion)
    pub fn find_block(&self, id: &str) -> Option<Vec<u64>> {
        let hash = self.index64.as_ref().unwrap().get_hash(id);

        for (i, j) in self
            .index
            .iter()
            .map(|x| x.0)
            .collect::<Vec<u64>>()
            .windows(2)
            .into_iter()
            .enumerate()
        {
            if j[0] <= hash && hash < j[1] {
                return Some(vec![self.index[i].1]);
            }
        }

        if self.index[self.index.len() - 1].0 >= hash {
            return Some(vec![self.index[self.index.len() - 1].1])
        } 
        None
    }

    pub fn plan_from_parts<'a>(
        hashes: &'a [u64],
        _locs: &[Bitpacked],
        hash_type: Hashes,
        min_size: u32,
    ) -> (StoredIndexPlan, Vec<&'a [u64]>) {
        assert!(
            hashes[..].is_sorted(),
            "Hashes Vector must be sorted. Did you forget to finalize the index?"
        );

        assert!(hashes.len() <= u64::MAX as usize, "Hashes Vector must be smaller than 2^64... Contact Joseph to Discuss options or split into multiple files...");

        let hashes_count = hashes.len();
        let mut parts = 64;
        let mut chunk_size = (hashes_count as f64 / parts as f64).ceil() as u32;

        if hashes.len() < MINIMUM_CHUNK_SIZE as usize {
            parts = 1;
            chunk_size = MINIMUM_CHUNK_SIZE;
        } else {
            while chunk_size < MINIMUM_CHUNK_SIZE {
                parts -= 1;
                chunk_size = (hashes_count as f64 / parts as f64).ceil() as u32;
                if parts == 1 {
                    chunk_size = MINIMUM_CHUNK_SIZE;
                    break;
                }
            }
        }

        let mut index = Vec::new();
        let mut hash_splits = Vec::new();

        for i in 0..parts {
            let start = i as usize * chunk_size as usize;
            let mut end = (i as usize + 1) * chunk_size as usize;
            end = std::cmp::min(end, hashes.len());

            index.push((hashes[start], 0)); // 0 here is a placeholder!
            hash_splits.push(&hashes[start..end]);
        }

        // Bitpacked locs are already split into small chunks, so we can process that in the format.rs file
        (
            StoredIndexPlan {
                parts,
                index,
                min_size,
                hash_type,
                chunk_size,
                index_len: hashes.len() as u32,
                index64: None,
            },
            hash_splits,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn test_index64() {
        let mut i64 = Index64::with_capacity(64);
        for n in 0..256 {
            let id = format!("test{}", n);
            i64.add(&id, n).expect("Unable to add to index");
        }

        i64.add("duplicate", 1001).expect("Unable to add to index");
        i64.add("duplicate", 1002).expect("Unable to add to index");
        i64.add("duplicate", 1003).expect("Unable to add to index");
        i64.add("duplicate", 1004).expect("Unable to add to index");
        i64.add("duplicate", 1005).expect("Unable to add to index");

        let mut i64 = i64.finalize();
        let ids = i64.ids.take().unwrap();

        let y = i64.find("test32").unwrap();
        assert!(i64.locs[y[0]] == 32);

        assert!(ids[y[0]] == "test32");

        let y = i64.find("not-in-the-index");
        assert!(y == None);

        let y = i64.find("duplicate");
        println!("{:#?}", y);
        let y = y.expect("Index did not find correctly.");
        let y = y.iter().map(|&x| i64.locs[x]).collect::<Vec<u32>>();
        assert!(y.len() == 5);
        assert!(y.contains(&1001));
        assert!(y.contains(&1002));
        assert!(y.contains(&1003));
        assert!(y.contains(&1004));
        assert!(y.contains(&1005));
    }

    #[test]
    pub fn test_index64_len() {
        let mut i64 = Index64::with_capacity(64);
        for n in 0..512 {
            let id = format!("test{}", n);
            i64.add(&id, n).expect("Unable to add to index");
        }

        for n in 0..512 {
            let id = "test";
            i64.add(&id, n).expect("Unable to add to index");
        }

        let i64 = i64.finalize();
        assert!(i64.len() == 1024);
    }
}

// Doesn't work for some reason (input data too small? or too repetitive in my tests?)
// Keeping in case of future attempts...
/*
fn zstd_train_dict_ids(ids: &Vec<String>) -> Vec<u8> {
    let bytes: Vec<u8> = ids
        .iter()
        .map(|x| x.as_bytes().to_owned())
        .flatten()
        .collect();
    // let bytes: Vec<Vec<u8>> = ids.iter().map(|x| x.as_bytes().to_owned()).collect();
    let lens: Vec<usize> = ids.iter().map(|x| x.len()).collect();

    // zstd::dict::from_samples(&bytes, 256).expect("Unable to create dictionary from IDs")
    zstd::dict::from_continuous(&bytes, &lens, 1024).expect("Unable to create dictionary from IDs")
} */

/* Not in use (yet?)
#[non_exhaustive]
#[derive(Serialize, Deserialize)]
enum IndexStored {
    Index32(Index32),
    Index64(Index64),
}

#[derive(Serialize, Deserialize)]
pub struct Index32 {
    hashes: Vec<u32>,
    locs: Vec<u32>,

    #[serde(skip)]
    pub ids: Option<Vec<String>>,
}

impl IDIndexer for Index32 {
    fn ids_chunks(&self, chunk_size: usize) -> Chunks<'_, std::string::String> {
        self.ids.as_ref().unwrap().chunks(chunk_size)
    }

    fn with_capacity(capacity: usize) -> Self {
        Index32 {
            hashes: Vec::new(),
            locs: Vec::with_capacity(capacity),
            ids: Some(Vec::with_capacity(capacity)),
        }
    }

    fn new() -> Self {
        Index32 {
            hashes: Vec::new(),
            locs: Vec::new(),
            ids: Some(Vec::new()),
        }
    }

    fn add(&mut self, id: &str, loc: u32) -> Result<(), &'static str> {
        // TODO: Hasing fn, lowercase stuff...
        let mut hasher = XxHash32::with_seed(42);
        hasher.write(id.as_bytes());
        let hash = hasher.finish();
        // XxHash32 outputs a 64 bit, with the first 32 bits being 0
        // So this transmute is just fine... we just want to drop those first bits...
        let hash: [u32; 2] = unsafe { std::mem::transmute(hash) };

        self.hashes.push(hash[1]);
        self.locs.push(loc);
        self.ids.as_mut().unwrap().push(id.to_string());

        Ok(())
    }

    fn find(&self, id: &str) -> Option<Vec<usize>> {
        let mut hasher = XxHash32::with_seed(42);
        hasher.write(id.as_bytes());
        let hash = hasher.finish();
        let hash: [u32; 2] = unsafe { std::mem::transmute(hash) };
        let hash = hash[1];

        let found = match self.hashes.binary_search(&hash) {
            Ok(x) => x,
            Err(_) => return None,
        };

        let mut locs = Vec::new();

        if self.hashes[found - 1] != hash && self.hashes[found + 1] != hash {
            // locs.push(self.locs[found]);
            locs.push(found);
            return Some(locs);
        }

        let mut start = found;
        let mut end = found;

        while self.hashes[start] == hash {
            start = start.saturating_sub(1);
        }

        start = start.saturating_add(1);

        let len = self.locs.len();

        while self.hashes[end] == hash && end < len {
            end = end.saturating_add(1);
        }

        end = end.saturating_sub(1);

        Some((start..=end).collect())
    }

    fn finalize(self) -> Self {
        // TODO: More memory efficient way...
        // But this is a one-time cost so it's hard to justify spending much time or pulling in other crates...

        let mut tuples: Vec<(u32, u32, String)> = Vec::with_capacity(self.locs.len());

        for i in 0..self.locs.len() {
            tuples.push((
                self.hashes[i],
                self.locs[i],
                self.ids.as_ref().unwrap()[i].clone(),
            ))
        }

        tuples.sort_by(|a, b| a.0.cmp(&b.0));

        /* let mut tuples = self
            .hashes
            .into_iter()
            .zip(self.locs.into_iter())
            // .zip(self.ids.into_iter())
            .collect::<Vec<(u32, u64)>>();
        tuples.sort_by(|a, b| a.0.cmp(&b.0)); */
        let hashes = tuples.iter().map(|(i, _, _)| *i).collect::<Vec<u32>>();
        let locs = tuples.iter().map(|(_, o, _)| *o).collect::<Vec<u32>>();
        let ids = tuples
            .iter()
            .map(|(_, _, x)| x.clone())
            .collect::<Vec<String>>();

        Index32 {
            hashes,
            locs,
            ids: Some(ids),
        }
    }

    fn len(&self) -> u64 {
        self.locs.len() as u64
    }

    fn is_empty(&self) -> bool {
        self.locs.len() == 0
    }

    fn set_ids(&mut self, ids: Vec<String>) {
        assert!(ids.len() == self.locs.len());
        self.ids = Some(ids);
    }
}
*/
