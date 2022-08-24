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

#[derive(PartialEq, Eq, Clone, Copy, bincode::Encode, bincode::Decode, Debug)]
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
    pub hashes: Vec<u64>,
    pub ids: Vec<String>,
    pub locs: Vec<u32>, // To be sequentially stored on disk in chunks, so that only Vec<u64> is decompressed
}

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
            hashes: Vec::with_capacity(len),
            ids: Vec::with_capacity(len),
            locs: Vec::with_capacity(len),
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

        writer.hashes = hashes;
        writer.ids = ids;
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

        let len = self.hashes.len();
        let chunks = (len as f64 / (self.chunk_size as usize) as f64).ceil() as usize;
        let chunk_size = self.chunk_size as usize;

        let hash_index: Vec<u64> = (0..chunks).map(|i| self.hashes[i * chunk_size]).collect();

        // Write a dummy index header
        let header_loc = out_buf.seek(SeekFrom::Current(0)).unwrap();
        bincode::encode_into_std_write(&len, &mut out_buf, bincode_config).unwrap();
        bincode::encode_into_std_write(&self.hasher, &mut out_buf, bincode_config).unwrap();
        bincode::encode_into_std_write(&self.chunk_size, &mut out_buf, bincode_config).unwrap(); // Chunk size
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
        let mut hash_block_index = hash_index
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

        for i in 0..chunks {
            // Below is 136.71ms
            let start = i * chunk_size;
            let end = std::cmp::min((i + 1) * chunk_size, len);
            hash_block_index[i].1 = out_buf.seek(SeekFrom::Current(0)).unwrap();
            bincode::encode_into_std_write(
                &self.hashes[start..end].to_vec(),
                &mut out_buf,
                bincode_config,
            )
            .expect("Bincode error");
        }

        // Write the hash chunks
        // Current written data is:
        // 0 # u64 hash_index
        // 0 # u64 bitpack location
        // [(123, 0, 0), (234, 0, 0), (345, 0, 0), ...] -- key/id block location index (key here is the first hash value in the block)
        // [[u64, u64, u64, u64...], [u64, u64, u64, u64...],...] // Blocks of u64 hashes...
        // [Vec<u8>, Vec<u8>, Vec<u8>, ...] // Blocks of compressed (lz4_flex) Strings (IDs)
        for i in 0..chunks {
            let start = i * chunk_size;
            let end = std::cmp::min((i + 1) * chunk_size, len);

            hash_block_index[i].2 = out_buf.seek(SeekFrom::Current(0)).unwrap();

            let uncompressed =
                bincode::encode_to_vec(&self.ids[start..end].to_vec(), bincode_config).unwrap();
            let compressed: Vec<u8> = Vec::with_capacity(2 * 1024 * 1024);
            let mut encoder = zstd::stream::Encoder::new(compressed, 5).unwrap();
            encoder.include_magicbytes(false)
                   .expect("Unable to set ZSTD MagicBytes");
            encoder.write_all(&uncompressed).unwrap();
            // let compressed = lz4_flex::compress_prepend_size(&uncompressed);
            let compressed = encoder.finish().unwrap();
            bincode::encode_into_std_write(&*compressed, &mut out_buf, bincode_config)
                .expect("Bincode error");
        }

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
        bincode::encode_into_std_write(&len, &mut out_buf, bincode_config).unwrap();
        bincode::encode_into_std_write(&self.hasher, &mut out_buf, bincode_config).unwrap();
        bincode::encode_into_std_write(&self.chunk_size, &mut out_buf, bincode_config).unwrap(); // Chunk size
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
        //let locs: Vec<u32> = self
        //.locs
        //.iter()
        //.map(|x| *x - *self.locs.iter().min().unwrap() as u32)
        //.collect();

        bitpack_u32(&self.locs)
    }
}

#[derive(Debug)]
pub struct DualIndex {
    pub idx_start: u64,
    pub chunk_size: u64,
    pub hash_index: u64,
    pub bitpacked_loc: u64,
    pub num_bits: u8,
    pub bitpacked_len: u64,
    pub remainder_loc: u64,
    pub value_block_index: Option<Vec<(u64, u64, u64)>>,
    pub hasher: Hashes,
    pub len: usize,
}

impl DualIndex {
    pub fn new<R>(mut in_buf: &mut R, idx_start: u64) -> Self
    where
        R: Read + Seek,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        in_buf.seek(SeekFrom::Start(idx_start)).unwrap();
        let len: usize = bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();
        let hasher: Hashes = bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();
        let chunk_size: u64 = bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();
        let hash_index = bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();
        let bitpacked_loc = bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();
        let num_bits = bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();
        let bitpacked_len = bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();
        let remainder_loc = bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();

        DualIndex {
            idx_start,
            chunk_size,
            hash_index,
            bitpacked_loc,
            num_bits,
            bitpacked_len,
            remainder_loc,
            value_block_index: None,
            hasher,
            len,
        }
    }

    pub fn load_index<R>(&mut self, mut in_buf: &mut R)
    where
        R: Read + Seek,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();
        in_buf.seek(SeekFrom::Start(self.hash_index)).unwrap();
        let hash_block_index: Vec<(u64, u64, u64)> =
            bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();
        self.value_block_index = Some(hash_block_index);
    }

    pub fn all_ids<R>(&mut self, mut in_buf: &mut R) -> Vec<String>
    where
        R: Read + Seek,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        if self.value_block_index.is_none() {
            self.load_index(&mut in_buf);
        }

        let mut ids = Vec::with_capacity(
            self.value_block_index.as_ref().unwrap().len() * self.chunk_size as usize,
        );

        for (_, _, pos) in self.value_block_index.as_ref().unwrap().iter() {
            in_buf.seek(SeekFrom::Start(*pos as u64)).unwrap();
            let compressed: Vec<u8> =
                bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();
            
            // let decompressed = lz4_flex::decompress_size_prepended(&compressed).unwrap();
            let mut decoder = zstd::stream::Decoder::new(&compressed[..]).unwrap();
            decoder
                .include_magicbytes(false)
                .expect("Unable to disable magicbytes in decoder");
            let mut decompressed: Vec<u8> = Vec::with_capacity(2 * 1024 * 1024);
            decoder.read_to_end(&mut decompressed).unwrap();
            let ids_in_chunk: Vec<String> =
                bincode::decode_from_slice(&decompressed[..], bincode_config)
                    .unwrap()
                    .0;
            ids.extend(ids_in_chunk);
        }

        ids
    }

    fn get_putative_block(&self, hash: u64) -> Option<(usize, u64, u64)> {
        let vbi = self.value_block_index.as_ref().unwrap();
        if vbi.is_empty() {
            return Some((0, vbi[0].1, vbi[0].2));
        }

        // TODO: Make this a better binary search....
        // TODO: It looks like Vec's binary_search_by would give us the correct answer!

        let find = vbi.binary_search_by(|&(h, _, _)| {
            h.cmp(&hash)
            /*            match h.cmp(&hash) {
                Ordering::Equal => Ordering::Equal,
                std::Ordering::Less => Ordering::Greater,
                std::cmp::Ordering::Greater => Ordering::Less,
            }
            if h > hash {
                std::cmp::Ordering::Greater
            } else if h < hash {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            } */
        });

        match find {
            Ok(i) => Some((i, vbi[i].1, vbi[i].2)),
            Err(i) => {
                if i == 0 {
                    Some((0, vbi[0].1, vbi[0].2))
                } else {
                    Some((i - 1, vbi[i - 1].1, vbi[i - 1].2))
                }
            }
        }
    }

    fn get_hash_chunk_by_loc<R>(&self, mut in_buf: R, loc: u64) -> Vec<u64>
    where
        R: Read + Seek,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();
        in_buf.seek(SeekFrom::Start(loc)).unwrap();
        bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap()
    }

    fn get_id_chunk_by_loc<R>(&self, mut in_buf: R, loc: u64) -> Vec<String>
    where
        R: Read + Seek,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();
        in_buf.seek(SeekFrom::Start(loc)).unwrap();
        let compressed: Vec<u8> =
            bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();

        let mut decoder = zstd::stream::Decoder::new(&compressed[..]).unwrap();
        decoder
            .include_magicbytes(false)
            .expect("Unable to disable magicbytes in decoder");

        let mut decompressed: Vec<u8> = Vec::with_capacity(2 * 1024 * 1024);
        decoder.read_to_end(&mut decompressed).unwrap();
        // let decompressed = lz4_flex::decompress_size_prepended(&compressed).unwrap();

        bincode::decode_from_slice(&decompressed[..], bincode_config)
            .unwrap()
            .0
    }

    pub fn get_bitpacked_chunk_by_pos<R>(&self, mut in_buf: R, pos: usize) -> Packed
    where
        R: Read + Seek,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        let chunk = (pos / BitPacker8x::BLOCK_LEN) as u64;
        let bitpacked_loc = self.bitpacked_loc as u64 + (chunk * self.bitpacked_len);
        if bitpacked_loc > self.remainder_loc {
            in_buf.seek(SeekFrom::Start(self.remainder_loc)).unwrap();
        } else {
            in_buf.seek(SeekFrom::Start(bitpacked_loc)).unwrap();
        }

        let chunk: Packed = bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();

        return chunk;
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn find<R>(&mut self, mut in_buf: &mut R, id: &str) -> Option<u32>
    where
        R: Read + Seek,
    {
        if self.value_block_index.is_none() {
            self.load_index(&mut in_buf);
        }

        let hash = self.hasher.hash(id);

        let putative_block = self.get_putative_block(hash);

        if putative_block.is_none() {
            return None;
        }

        let (block_num, hash_chunk_loc, id_chunk_loc) = putative_block.unwrap();
        let hash_chunk = self.get_hash_chunk_by_loc(&mut in_buf, hash_chunk_loc);
        let pos = hash_chunk.binary_search(&hash);

        // Hash is not found in the chunk, return...
        if pos.is_err() {
            return None;
        }

        // If hash is found, we unpack the actual ID's...
        let id_chunk = self.get_id_chunk_by_loc(&mut in_buf, id_chunk_loc);

        if id_chunk[pos.unwrap() as usize] != id {
            return None;
        }

        let actual_loc = pos.unwrap() as usize + block_num * self.chunk_size as usize;
        let bp = self.get_bitpacked_chunk_by_pos(&mut in_buf, actual_loc);
        let value = bp.unpack(self.num_bits);

        let bitpacked_pos = (actual_loc % BitPacker8x::BLOCK_LEN);

        Some(value[bitpacked_pos])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::Rng;
    use std::io::Cursor;

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
    }

    #[test]
    pub fn test_dual_index_reader() {
        let mut out_buf: Cursor<Vec<u8>> = Cursor::new(Vec::new());
        let mut di = DualIndexBuilder::new();

        let mut rng = rand::thread_rng();

        for i in 0_u64..10000 {
            di.add(i.to_string(), rng.gen::<u32>());
        }

        di.add("Max".to_string(), std::u32::MAX);

        let mut writer: DualIndexWriter = di.into();
        writer.write_to_buffer(&mut out_buf);
        drop(writer);

        let mut in_buf = Cursor::new(out_buf.into_inner());

        let mut reader = DualIndex::new(&mut in_buf, 0);

        let ids = reader.all_ids(&mut in_buf);
        assert_eq!(ids.len(), 10001);

        assert_eq!(Some(std::u32::MAX), reader.find(&mut in_buf, "Max"));

        // TODO: Some more tests....
    }
}
