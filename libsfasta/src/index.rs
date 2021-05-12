use serde::{Deserialize, Serialize};
use twox_hash::{XxHash32, XxHash64, Xxh3Hash64};
use std::hash::Hasher;
use ahash::{AHasher, RandomState};

extern crate serde;

#[non_exhaustive]
enum IndexTypes {
    Index32,
    Index64,
}

// TODO: Implement small hasher (HashMap, HashBrown) for very small datasets
// TODO: Implement 16-bit hasher

pub trait IDIndexer {
    fn add(&mut self, id: &str, loc: u64) -> Result<(), &'static str>;

    // We return a vector of possible matches
    // We deal with collisions by not-dealing with collisions
    // It's fast enough to take the list of candidates, and query them directly...
    fn find(&mut self, id: &str) -> Option<Vec<u64>>;

    // Finalize (no more additions)
    // This function should handle the sorting (binary search doesn't work without it)
    fn finalize(self) -> Self;
}

#[derive(Serialize, Deserialize)]
pub struct Index32 {
    hashes: Vec<u32>,
    locs: Vec<u64>,
    // ids: Vec<String>,
}

impl IDIndexer for Index32 {
    fn add(&mut self, id: &str, loc: u64) -> Result<(), &'static str> {
        let mut hasher = XxHash32::with_seed(42);
        hasher.write(id.as_bytes());
        let hash = hasher.finish();
        // XxHash32 outputs a 64 bit, with the first 32 bits being 0
        // So this transmute is just fine... we just want to drop those first bits...
        let hash: [u32; 2] = unsafe { std::mem::transmute(hash) };

        self.hashes.push(hash[1]);
        self.locs.push(loc);
        // self.ids.push(id.to_string());

        Ok(())
    }

    fn find(&mut self, id: &str) -> Option<Vec<u64>> {
        let mut hasher = XxHash32::with_seed(42);
        hasher.write(id.as_bytes());
        let hash = hasher.finish();
        let hash: [u32; 2] = unsafe { std::mem::transmute(hash) };
        let hash = hash[1];

        let found = match self.hashes.binary_search(&hash) {
            Ok(x) => x,
            Err(_) => return None
        };

        let mut locs = Vec::new();
        
        if self.hashes[found-1] != hash && self.hashes[found+1] != hash {
            locs.push(self.locs[found]);
            return Some(locs);
        }

        let mut keepexpanding = true;
        let mut start = found;
        let mut end = found;
        
        while self.hashes[start] == hash {
            start = start.saturating_sub(1);
        }

        let len = self.hashes.len();

        while self.hashes[end] == hash && end < len {
            start = start.saturating_add(1);
        }

        Some(self.locs[start..=end].to_vec())
    }

    fn finalize(self) -> Self {
        // TODO: More memory efficient way...
        // But this is a one-time cost so it's hard to justify spending much time or pulling in other crates...

        let mut tuples = self.hashes.into_iter().zip(self.locs.into_iter()).collect::<Vec<(u32, u64)>>();
        tuples.sort_by(|a, b| a.0.cmp(&b.0));
        let hashes = tuples.iter().map(|(i, o)| *i).collect::<Vec<u32>>();
        let locs = tuples.iter().map(|(i, o)| *o).collect::<Vec<u64>>();
        Index32 { hashes, locs }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
enum Hashes {
    Ahash, // ahash
    XxHash64,
    Xxh3Hash64, // This is not a stable hash right now. Here for future-proofing a bit...
}

#[derive(Serialize, Deserialize)]
pub struct Index64 {
    hashes: Vec<u64>,
    locs: Vec<u64>,
    hash: Hashes,
    // ids: Vec<String>,
}

impl Default for Index64 {
    fn default() -> Index64 {
        Index64 {
            hashes: Vec::new(),
            locs: Vec::new(),
            hash: Hashes::Ahash,
        }
    }
}

impl Index64 {
    fn get_hash(&mut self, id: &str) -> u64 {
        // TODO: Pretty sure this code could be simplified with dyn Hasher trait...
        // Not sure if a Box<> overhead would be worth it though...

        let hash;
        if self.hash == Hashes::Ahash {
            let mut hasher = AHasher::new_with_keys(42, 1010);
            hasher.write(id.as_bytes());
            hash = hasher.finish();
        } else if self.hash == Hashes::Xxh3Hash64 {
            let mut hasher = Xxh3Hash64::with_seed(42);
            hasher.write(id.as_bytes());
            hash = hasher.finish();
        } else {
            let mut hasher = XxHash64::with_seed(1010);
            hasher.write(id.as_bytes());
            hash = hasher.finish();
        }
        hash
    }
}

impl IDIndexer for Index64 {
    fn add(&mut self, id: &str, loc: u64) -> Result<(), &'static str> {
        let hash = self.get_hash(id);

        self.hashes.push(hash);
        self.locs.push(loc);
        // self.ids.push(id.to_string());

        Ok(())
    }

    // TODO: Dedupe this code with above...
    fn find(&mut self, id: &str) -> Option<Vec<u64>> {
        let hash = self.get_hash(id);

        let found = match self.hashes.binary_search(&hash) {
            Ok(x) => x,
            Err(_) => return None
        };

        let mut locs = Vec::new();
        
        if self.hashes[found-1] != hash && self.hashes[found+1] != hash {
            locs.push(self.locs[found]);
            return Some(locs);
        }

        let mut keepexpanding = true;
        let mut start = found;
        let mut end = found;
        
        while self.hashes[start] == hash {
            start = start.saturating_sub(1);
        }

        let len = self.hashes.len();

        while self.hashes[end] == hash && end < len {
            start = start.saturating_add(1);
        }

        Some(self.locs[start..=end].to_vec())
    }

    fn finalize(self) -> Self {
        // TODO: More memory efficient way...
        // But this is a one-time cost so it's hard to justify spending much time or pulling in other crates...
        let mut tuples = self.hashes.into_iter().zip(self.locs.into_iter()).collect::<Vec<(u64, u64)>>();
        tuples.sort_by(|a, b| a.0.cmp(&b.0));
        let hashes = tuples.iter().map(|(i, o)| *i).collect::<Vec<u64>>();
        let locs = tuples.iter().map(|(i, o)| *o).collect::<Vec<u64>>();
        Index64 { hashes, locs, hash: self.hash }
    }
}