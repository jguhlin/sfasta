//! Dual Index
//!
//! The dual index is a two-tiered index to speed up large datasets.
//!
//! Entries are effectively String -> u64
//! Which is Sequence ID -> byte location of SeqLoc Entry
//!
//! This probably poorly named dual-layered index converts to on-disk storage as such:
//!
//! IDs are lower-cased and Hashed with XxHash64 (referred to as hash)
//! IDs are then sorted by their hash value
//! IDs are stored in CHUNK_SIZE chunks
//! Hashes are stored in CHUNK_SIZE chunks
//! Hash index is stored as a Vec<(Hash, Loc)>
//! Where the Hash is the u64 of the first hash of the chunk, and Loc is the u64 byte location of the ordinal Hashes chunk
//!
//! A search is performed by identifying which chunk contains the hash, by finding index[n] <= HASH_QUERY <= index[n+1].0
//! Possible matches can span multiple chunks (IDs do not need to be unique).
//!
//! The Hash chunk *n* is then opened and searches for an exact match happen.
//!
//! If exact match(es) of the hash are found, the ID chunks *n* are opened, using the ordinal position of the exact match hashes, the Strings are compared.
//! If matched, then a match is successful.
//!
//! The important storage types are:
//! HashIndex -> Vec<(Hash, Hash Chunk Loc)> which is Vec<(u64, u64)>
//! HashChunks -> sequentially stored as Vec<Hash> which is Vec<u64>
//! IDChunks -> sequentially stored as Vec<lowercase ID> which is Vec<String>
//! LocChunks -> sequentially stored as chunks of Vec<u64> but bitpacked for fast decompression
//! HashChunkLocs -> Vec<u64> where each HashChunk is stored on disk (bitpacked)
//! IDChunkLocs -> Vec<u64> where each IDChunk is stored on disk (bitpacked)
//! LocChunks -> Vec<u64> where each Loc chunk is stored on disk (bitpacked)
//! Where SeqLoc ordinal (u64) of the SeqLoc entry (To be calculated from SeqLoc blocks, which are also chunked, but outside the scope of this index)
//!
//! In order to populate the Locs properly, the data is written to the file in the reverse order, thus:
//! IDChunks followed by HashChunks followed by HashIndex

use std::convert::TryFrom;
use std::hash::Hasher;
use std::io::{Read, Seek, SeekFrom, Write};

use crate::utils::{bitpack_u32, Bitpacked, Packed};

use bitpacking::{BitPacker, BitPacker8x};
use bumpalo::Bump;
use rayon::prelude::*;
use twox_hash::{XxHash64, Xxh3Hash64};

const DEFAULT_CHUNK_SIZE: u64 = 2048;

// TODO: Test different values of...
const MULTITHREAD_BOUNDARY: usize = 8 * 1024 * 1024;

#[derive(PartialEq, Eq, Clone, Copy, bincode::Encode, bincode::Decode)]
pub enum Hashes {
    XxHash64,
    Xxh3Hash64,
}

impl Hashes {
    pub fn hash(&self, id: &str) -> u64 {
        match self {
            Hashes::XxHash64 => {
                let mut hasher = XxHash64::with_seed(42);
                hasher.write(id.as_bytes());
                hasher.finish()
            }
            Hashes::Xxh3Hash64 => {
                let mut hasher = Xxh3Hash64::with_seed(42);
                hasher.write(id.as_bytes());
                hasher.finish()
            }
        }
    }
}

pub struct DualIndexBuilder {
    pub ids: Vec<String>,
    pub locs: Vec<u32>,
    pub chunk_size: u64,
    pub hasher: Hashes,
}

impl DualIndexBuilder {
    pub fn new() -> Self {
        Self {
            ids: Vec::new(),
            locs: Vec::new(),
            chunk_size: DEFAULT_CHUNK_SIZE,
            hasher: Hashes::XxHash64,
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            ids: Vec::with_capacity(capacity),
            locs: Vec::with_capacity(capacity),
            chunk_size: DEFAULT_CHUNK_SIZE,
            hasher: Hashes::XxHash64,
        }
    }

    pub fn with_hash(mut self, hash: Hashes) -> Self {
        self.hasher = hash;
        self
    }

    pub fn with_chunk_size(mut self, chunk_size: u64) -> Self {
        self.chunk_size = chunk_size;
        self
    }

    /// Add an entry to the index.
    ///
    /// id: String ID
    /// loc: u32 location of the SeqLoc entry in the file
    pub fn add(&mut self, id: String, loc: u32) {
        self.ids.push(id);
        self.locs.push(loc);
    }
}

/// Before written to file version
pub struct DualIndexWriter {
    pub chunk_size: u64,
    pub hasher: Hashes,
    pub hash_index: Vec<u64>,
    pub hash_chunks: Vec<Vec<u64>>, // To be sequentially stored on disk in chunks, so that only the needed Vec<u64> is decompressed
    pub id_chunks: Vec<Vec<String>>, // To be sequentially stored on disk in chunks, so that only Vec<String> is decompressed
    pub locs: Vec<u32>, // To be sequentially stored on disk in chunks, so that only Vec<u64> is decompressed
    pub hash_chunk_locs: Vec<u64>, // To be sequentially stored on disk in chunks, so that only Vec<u64> is decompressed
    pub id_chunk_locs: Vec<u64>, // To be sequentially stored on disk in chunks, so that only Vec<u64> is decompressed
    pub loc_chunk_locs: Vec<u64>, // To be sequentially stored on disk in chunks, so that only Vec<u64> is decompressed
}

// TODO: Make chunk size an option
impl From<DualIndexBuilder> for DualIndexWriter {
    fn from(builder: DualIndexBuilder) -> Self {
        let len = builder.ids.len();

        assert!(len <= u32::MAX as usize, "u32::MAX is the maximum number of sequences. This can be addressed if necessary, please contact Joseph directly to discuss.");

        let DualIndexBuilder {
            ids,
            locs,
            chunk_size,
            hasher,
        } = builder;

        let mut writer = DualIndexWriter {
            hasher,
            chunk_size: builder.chunk_size,
            hash_index: Vec::with_capacity(len),
            hash_chunks: Vec::with_capacity(len),
            id_chunks: Vec::with_capacity(len),
            locs: Vec::new(),
            hash_chunk_locs: Vec::with_capacity(len),
            id_chunk_locs: Vec::with_capacity(len),
            loc_chunk_locs: Vec::with_capacity(len),
        };

        // Only multithread hashing when the data is relatively large...
        let hashes = if len >= MULTITHREAD_BOUNDARY {
            ids.par_iter()
                .map(|id| writer.hasher.hash(id))
                .collect::<Vec<u64>>()
        } else {
            ids.iter()
                .map(|id| writer.hasher.hash(id))
                .collect::<Vec<u64>>()
        };

        let mut tuples: Vec<(u64, u32, String)> = izip!(hashes, locs, ids).collect();
        if len >= MULTITHREAD_BOUNDARY {
            tuples.par_sort_unstable_by(|a, b| a.0.cmp(&b.0));
        } else {
            tuples.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        }

        let mut hashes: Vec<u64> = Vec::with_capacity(len);
        let mut locs: Vec<u32> = Vec::with_capacity(len);
        let mut ids: Vec<String> = Vec::with_capacity(len);

        for (hash, loc, id) in tuples.into_iter() {
            //tuples.drain(..) {
            hashes.push(hash);
            locs.push(loc);
            ids.push(id);
        }

        let chunks = (len as f64 / (chunk_size as usize) as f64).ceil() as usize;

        let chunk_size = chunk_size as usize;

        // TODO: Put into chunks instead of .to_vec() which is likely going to make a copy...
        for i in 0..chunks {
            let start = i * chunk_size;
            let mut end = std::cmp::min((i + 1) * chunk_size, len);

            writer.hash_index.push(hashes[i * chunk_size]);
            writer.hash_chunks.push(hashes[start..end].to_vec());
            writer.id_chunks.push(ids[start..end].to_vec());
        }

        writer.locs = locs;

        writer
    }
}

impl DualIndexWriter {
    // Writes the dual index to a buffer (usually a file)
    pub fn write_to_buffer<W>(&mut self, mut out_buf: &mut W)
    where
        W: Write + Seek,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        // Write a dummy index header
        let header_loc = out_buf.seek(SeekFrom::Current(0)).unwrap();
        bincode::encode_into_std_write(&(0_u64), &mut out_buf, bincode_config).unwrap(); // hash_index
        bincode::encode_into_std_write(&(0_u64), &mut out_buf, bincode_config).unwrap(); // bitpacked location
        bincode::encode_into_std_write(&(0_u8), &mut out_buf, bincode_config).unwrap(); // num bits
        bincode::encode_into_std_write(&(0_u64), &mut out_buf, bincode_config).unwrap(); // bitpacked len
        bincode::encode_into_std_write(&(0_u64), &mut out_buf, bincode_config).unwrap(); // remainder loc


        // Write the locs_start

        // The starting loc for all is 0, because locs are just enumerated
        //bincode::encode_into_std_write(&self.locs_start, &mut out_buf, bincode_config)
        //    .expect("Bincode error");

        let hash_index_location = out_buf.seek(SeekFrom::Current(0)).unwrap();

        // Block Locs:
        // u64 of the first hash in the block, u64 of the location of the block of bincoded hashes...
        let mut hash_block_index = self
            .hash_index
            .iter()
            .map(|x| (*x, 0, 0))
            .collect::<Vec<(u64, u64, u64)>>();

        // Write the block_locs
        // Location of value blocks (value in this case are Locations)
        // Current written data is:
        // 0 # u64 hash_index
        // 0 # u64 bitpack location
        // [(123, 0, 0), (234, 0, 0), (345, 0, 0), ...]
        bincode::encode_into_std_write(&hash_block_index, &mut out_buf, bincode_config)
            .expect("Bincode error"); // this one is a dummy value

        // Write the hash chunks
        // Current written data is:
        // 0 # u64 hash_index
        // 0 # u64 bitpack location
        // [(123, 0, 0), (234, 0, 0), (345, 0, 0), ...] -- key/id block location index (key here is the first hash value in the block)
        // [[u64, u64, u64, u64...], [u64, u64, u64, u64...],...] // Blocks of u64 hashes...
        // Hashes don't really compress, so we don't try...
        for (i, chunk) in self.hash_chunks.iter().enumerate() {
            hash_block_index[i].1 = out_buf.seek(SeekFrom::Current(0)).unwrap();
            bincode::encode_into_std_write(&chunk, &mut out_buf, bincode_config)
                .expect("Bincode error");
        }

        // Write the hash chunks
        // Current written data is:
        // 0 # u64 hash_index
        // 0 # u64 bitpack location
        // [(123, 0, 0), (234, 0, 0), (345, 0, 0), ...] -- key/id block location index (key here is the first hash value in the block)
        // [[u64, u64, u64, u64...], [u64, u64, u64, u64...],...] // Blocks of u64 hashes...
        // [Vec<u8>, Vec<u8>, Vec<u8>, ...] // Blocks of compressed (lz4_flex) Strings (IDs)
        let bump = Bump::new();
        for (i, id_chunk) in self.id_chunks.iter().enumerate() {
            hash_block_index[i].2 = out_buf.seek(SeekFrom::Current(0)).unwrap();

            let uncompressed =
                bump.alloc(bincode::encode_to_vec(id_chunk, bincode_config).unwrap());
            println!("IDs uncompressed len: {}", uncompressed.len());
            let compressed = bump.alloc(lz4_flex::compress(uncompressed));
            println!("IDs compressed len: {}", compressed.len());
            bincode::encode_into_std_write(id_chunk, &mut out_buf, bincode_config)
                .expect("Bincode error");
        }
        drop(bump);

        // Current written data is:
        // 0 # u64 hash_index
        // 0 # u64 bitpack location
        // [(123, 0, 0), (234, 0, 0), (345, 0, 0), ...] -- key/id block location index (key here is the first hash value in the block)
        // [[u64, u64, u64, u64...], [u64, u64, u64, u64...],...] // Blocks of u64 hashes...
        // [Vec<u8>, Vec<u8>, Vec<u8>, ...] // Blocks of compressed (lz4_flex) Strings (IDs)
        // [Vec<u8>, Vec<u8>, Vec<u8>, ...] // Blocks of bitpacked u32 integers
        let (num_bits, bitpacked) = self.bitpack();
        let bitpacked_location = out_buf.seek(SeekFrom::Current(0)).unwrap();

        let mut bitpacked_len = 0;
        let mut remainder_loc: u64 = 0;

        // Write the bitpacked data
        for bp in bitpacked.into_iter() {
            remainder_loc = out_buf.seek(SeekFrom::Current(0)).unwrap();
            let len = bincode::encode_into_std_write(&bp, &mut out_buf, bincode_config)
                .expect("Bincode error");
            println!("Len: {}", len);
            if bitpacked_len == 0 && bp.is_packed() {
                bitpacked_len = len;
            } else if bp.is_packed() {
                assert_eq!(bitpacked_len, len);
            }
        }

        // Go back and write the correct index
        // Current data is:
        // 0 # u64 hash_index
        // 0 # u64 bitpack location
        // [(123, 1234, 543), (234, 2345, 432), (345, 3456, 321), (456, 4567, 584), ...] -- value block index, dummy values replaced with byte location
        // [[u64, u64, u64, u64...], [u64, u64, u64, u64...],...] // Blocks of u64 hashes...
        // [Vec<u8>, Vec<u8>, Vec<u8>, ...] // Blocks of compressed (lz4_flex) Strings (IDs)
        // [Vec<u8>, Vec<u8>, Vec<u8>, ...] // Blocks of bitpacked u32 integers
        // -> go back to this point once finished
        let end = out_buf.seek(SeekFrom::Current(0)).unwrap();
        out_buf.seek(SeekFrom::Start(hash_index_location)).unwrap();
        bincode::encode_into_std_write(&hash_block_index, &mut out_buf, bincode_config)
            .expect("Bincode error");
        out_buf.seek(SeekFrom::Start(end)).unwrap();

        // Go back and write the correct headerr
        let end = out_buf.seek(SeekFrom::Current(0)).unwrap();
        out_buf.seek(SeekFrom::Start(header_loc)).unwrap();
        bincode::encode_into_std_write(&hash_index_location, &mut out_buf, bincode_config).unwrap(); // value_block_index_location
        bincode::encode_into_std_write(&bitpacked_location, &mut out_buf, bincode_config).unwrap(); // hash_index
        bincode::encode_into_std_write(&num_bits, &mut out_buf, bincode_config).unwrap(); // num_bits
        bincode::encode_into_std_write(&bitpacked_len, &mut out_buf, bincode_config).unwrap(); // bitpacked_len
        bincode::encode_into_std_write(&remainder_loc, &mut out_buf, bincode_config).unwrap(); // remainder_loc

        // Go back to the end so we don't interfere with later operations...
        out_buf.seek(SeekFrom::Start(end)).unwrap();
    }

    fn bitpack(&self) -> (u8, Vec<Packed>) {
        // Subtract the starting location from all the locations.
        let locs: Vec<u32> = self
            .locs
            .iter()
            .map(|x| *x - *self.locs.iter().min().unwrap() as u32)
            .collect();

        bitpack_u32(&locs)
    }
}

pub struct DualIndex {
    pub locs_start: u64,
    pub locs: Vec<u64>,
    pub block_locs: Vec<u64>,
    pub blocks_locs_loc: u64,
    pub on_disk: bool,
}

impl DualIndex {
    pub fn new(locs_start: u64) -> Self {
        DualIndex {
            locs_start,
            locs: Vec::new(),
            block_locs: Vec::new(),
            blocks_locs_loc: u64::MAX,
            on_disk: false,
        }
    }

    pub fn bitpack(&self) -> (u8, Vec<Packed>) {
        // Subtract the starting location from all the locations.
        let locs: Vec<u64> = self.locs.iter().map(|x| x - self.locs_start).collect();

        // Assert that they can all fit into a u32
        assert!(locs.iter().max().unwrap() <= &(u32::MAX as u64), "Unexpected Edge case, too many IDs... please e-mail Joseph and I can fix this in the next release");

        // Convert them all to a u32
        let locs: Vec<u32> = locs
            .into_iter()
            .map(|x| u32::try_from(x).unwrap())
            .collect();

        bitpack_u32(&locs)
    }

    pub fn read_from_buffer<R>(mut in_buf: &mut R) -> Self
    where
        R: Read + Seek,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        let locs_start: u64 =
            bincode::decode_from_std_read(&mut in_buf, bincode_config).expect("Bincode error");
        let mut di = DualIndex::new(locs_start);
        di.on_disk = true;
        di.blocks_locs_loc =
            bincode::decode_from_std_read(&mut in_buf, bincode_config).expect("Bincode error");

        in_buf.seek(SeekFrom::Start(di.blocks_locs_loc)).unwrap();
        di.block_locs =
            bincode::decode_from_std_read(&mut in_buf, bincode_config).expect("Bincode error");

        // File position(seek) is at the end of the DualIndex block now...
        di
    }

    pub fn find_loc<R>(&self, mut buf: &mut R, pos: usize) -> u64
    where
        R: Read + Seek,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        let block_idx = pos / BitPacker8x::BLOCK_LEN;
        let block_inner_loc = pos % BitPacker8x::BLOCK_LEN;
        buf.seek(SeekFrom::Start(self.block_locs[block_idx]))
            .unwrap();

        let bp: Bitpacked =
            bincode::decode_from_std_read(&mut buf, bincode_config).expect("Bincode error");

        let block = bp.decompress();
        self.locs_start + block[block_inner_loc] as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use rand::Rng;

    #[test]
    pub fn test_dual_index_builder() {
        let mut out_buf: Cursor<Vec<u8>> = Cursor::new(Vec::new());
        let mut di = DualIndexBuilder::new();

        let mut rng = rand::thread_rng();

        for i in (0_u64..10000) {
            di.add(i.to_string(), (i * 2) as u32);
        }

        for i in (0_u64..10000).step_by(2) {
            di.add(i.to_string(), rng.gen::<u32>());
        }

        di.add("Max".to_string(), std::u32::MAX);

        for i in (0_u64..1000).step_by(2) {
            di.add(i.to_string(), rng.gen::<u32>());
        }

        let mut writer: DualIndexWriter = di.into();
        writer.write_to_buffer(&mut out_buf);
        println!("{:#?}", out_buf.into_inner().len());
        panic!();
    }
}
