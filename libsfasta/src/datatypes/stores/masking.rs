// NOTE: I've spent lots of time converting this to bitvec so that
// bool would be 1bit instead of 8bits (1 bytes) Compressed, this
// saves < 1Mbp on a 2.3Gbp uncompressed FASTA file... and triple the
// length in time for masking. TODO: Try stream vbytes for this...
use std::{
    io::{BufRead, Read, Seek, Write},
    pin::Pin,
    sync::Arc,
};

#[cfg(feature = "async")]
use bytes::{BufMut, Bytes, BytesMut};

#[cfg(feature = "async")]
use tokio_stream::StreamExt;

#[cfg(feature = "async")]
use async_stream::stream;

#[cfg(feature = "async")]
use tokio_stream::Stream;

#[cfg(feature = "async")]
use libfilehandlemanager::AsyncFileHandleManager;

use stream_vbyte::{
    decode::{cursor::DecodeCursor, decode},
    encode::encode,
    scalar::Scalar,
};

use vers_vecs::{BitVec, RsVec};

use super::Builder;
use crate::datatypes::{
    BlockStoreError, BytesBlockStore, BytesBlockStoreBuilder, Loc,
};
use libcompression::*;

#[cfg(feature = "async")]
use crate::parser::async_parser::{
    bincode_decode_from_buffer_async,
    bincode_decode_from_buffer_async_with_size_hint,
};

#[cfg(feature = "async")]
use tokio::{
    fs::File,
    io::{AsyncSeekExt, BufReader},
    sync::{OwnedRwLockWriteGuard, RwLock},
};

use pulp::Arch;

pub struct MaskingStoreBuilder
{
    pub inner: BytesBlockStoreBuilder,
}

impl Builder<Vec<u8>> for MaskingStoreBuilder
{
    fn add(&mut self, input: Vec<u8>) -> Result<Vec<Loc>, &str>
    {
        Ok(self.add(&input))
    }

    fn finalize(&mut self) -> Result<(), &str>
    {
        match self.inner.finalize() {
            Ok(_) => Ok(()),
            Err(e) => Err("Unable to finalize masking store"),
        }
    }
}

impl Default for MaskingStoreBuilder
{
    fn default() -> Self
    {
        MaskingStoreBuilder {
            inner: BytesBlockStoreBuilder::default()
                .with_block_size(512 * 1024),
        }
    }
}

impl MaskingStoreBuilder
{
    pub fn with_dict(mut self) -> Self
    {
        self.inner = self.inner.with_dict();
        self
    }

    pub fn with_dict_size(mut self, dict_size: u64) -> Self
    {
        self.inner = self.inner.with_dict_size(dict_size);
        self
    }

    pub fn with_dict_samples(mut self, dict_samples: u64) -> Self
    {
        self.inner = self.inner.with_dict_samples(dict_samples);
        self
    }

    pub fn with_compression(mut self, compression: CompressionConfig) -> Self
    {
        self.inner = self.inner.with_compression(compression);
        self
    }

    pub fn with_tree_compression(
        mut self,
        tree_compression: CompressionConfig,
    ) -> Self
    {
        self.inner = self.inner.with_tree_compression(tree_compression);
        self
    }

    pub fn len(&self) -> usize
    {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool
    {
        self.inner.is_empty()
    }

    pub fn write_header<W>(&mut self, pos: u64, mut out_buf: &mut W)
    where
        W: Write + Seek,
    {
        self.inner.write_header(pos, &mut out_buf);
    }

    pub fn write_block_locations<W>(
        &mut self,
        mut out_buf: W,
    ) -> Result<(), BlockStoreError>
    where
        W: Write + Seek,
    {
        self.inner.write_block_locations(&mut out_buf)
    }

    pub fn with_block_size(mut self, block_size: usize) -> Self
    {
        self.inner = self.inner.with_block_size(block_size);
        self
    }

    pub fn with_compression_worker(
        mut self,
        compression_worker: Arc<CompressionWorker>,
    ) -> Self
    {
        self.inner = self.inner.with_compression_worker(compression_worker);
        self
    }

    pub fn add(&mut self, seq: &[u8]) -> Vec<Loc>
    {
        // If none are lowercase, nope out here... Written in a way that
        // allows for easy vectorization for SIMD
        let arch = Arch::new();

        // Significant speedup with this...
        if arch.dispatch(|| {
            for x in seq.iter() {
                if x > &b'`' {
                    return false;
                }
            }
            true
        }) {
            return Vec::new();
        }

        if seq.len() == 0 {
            return Vec::new();
        }

        // No benefit to using pulp here... (even with for loop)
        // let masked: Vec<u8> =
        // seq.iter().map(|x| x > &b'`').map(|x| x as u8).collect();

        // RLE - while zstd does it, this can reduce some of the blocks to fit
        // into a single block
        // let masked = rle_encode(&masked);

        let masked =
            rle_encode(&seq.iter().map(|x| x > &b'`').collect::<Vec<bool>>());

        // let bincode_config =
        // bincode::config::standard().with_variable_int_encoding();

        // let masked = bincode::encode_to_vec(&masked,
        // bincode_config).unwrap();

        self.inner
            .add(masked)
            .expect("Failed to add masking to block store")
    }

    pub fn finalize(&mut self) -> Result<(), BlockStoreError>
    {
        self.inner.finalize()
    }
}

fn previous_rle_encode(data: &[u8]) -> Vec<(u64, u8)>
{
    let mut rle = Vec::new();
    let mut count = 0;
    let mut last = data[0];

    for x in data.iter() {
        if *x == last {
            count += 1;
        } else {
            rle.push((count, last));
            count = 1;
            last = *x;
        }
    }

    rle.push((count, last));
    rle
}

fn previous_rle_decode(rle: &[(u64, u8)]) -> Vec<u8>
{
    let mut data = Vec::new();

    for (count, value) in rle.iter() {
        for _ in 0..*count {
            data.push(*value);
        }
    }

    data
}

// New experimental one
// Instead of Vec<(u64, u8)> to convert back to [u8]
// Here we do
// First bit is starting state (0, or 1, for masked)
// Next number is RLE encoding of the first stage
// using modified integer storage (maybe a better way?) - stream vbyte
// maybe? So for numbers [0 1 1 0] represents a u4 of value 6
// But if the first bit is 1, it means a u8 is stored
// [1 0 0 0 0 0 0 0] with the first 4 bits the last 4 bits of u8 (so
// have to re-arrange) (hmmm, wonder how streaming stuff / packing
// handles it, maybe just use that directly)

// SO: for now, it's just u8's
// 0 values are set to 0
// So masking value of
// [1 1 1 1 1 1 1 1 1 1 0 0 0 0 0]
// Becomes -> [1 9 5]
// 1 as the first bit to indicate masking on, or initial value
// repeated is true then 5 0's (state switches with each integer)
// bit format is:
// [1 0 0 0 0 1 0 0 1 0 0 0 0 0 1 0 1]
//  _ initial bit
//    _______________ 9
//                    _______________ 5

// more notes
// store as stream vbyte?
// so initial bit is the state, then we have a stream of integers

fn rle_encode(data: &[bool]) -> Vec<u8>
{
    // let mut bit_vec = BitVec::new();

    // Initial state
    // bit_vec.append(data[0]);

    let mut compressed = Vec::new();
    compressed.push(data[0] as u8);

    // Where we are in the slice
    let mut idx = 1;
    let mut count: u32 = 0;
    let mut current_state = data[0];

    let mut integers = Vec::new();

    while idx < data.len() {
        if data[idx] == current_state {
            count += 1;
        } else {
            // If we have a count, we need to store it
            assert!(count > 0);
            assert!(count < u32::MAX);

            integers.push(count);

            // Switch state
            current_state = data[idx];
            count = 1;
        }

        idx += 1;
    }

    let mut encoded_data = Vec::new();
    encoded_data.resize(5 * integers.len(), 0);
    let encoded_len = encode::<Scalar>(&integers, &mut encoded_data);
    // log::trace!("Encoded {} u32s into {} bytes", integers.len(),
    // encoded_len);

    // Append
    assert!(integers.len() < u32::MAX as usize);
    let integers_count = integers.len() as u32;
    compressed.extend_from_slice(&integers_count.to_le_bytes());
    compressed.extend_from_slice(&encoded_data[..encoded_len]);

    compressed
}

fn rle_decode(rle: &[u8]) -> Vec<bool>
{
    let mut masking = Vec::new();

    let mut current_state = rle[0] == 1;
    let len = u32::from_le_bytes([rle[1], rle[2], rle[3], rle[4]]) as usize;
    let mut decoded_nums = Vec::new();
    decoded_nums.resize(len, 0);
    // log::trace!("Decoding {} bytes into {} u32s", rle.len() - 5, len);
    let bytes_decoded = decode::<Scalar>(&rle[5..], len, &mut decoded_nums);
    // log::trace!("Decoded {} bytes into {} u32s", bytes_decoded, len);

    masking.push(current_state);

    for x in decoded_nums.iter() {
        for _ in 0..*x {
            masking.push(current_state);
        }

        current_state = !current_state;
    }

    masking
}

pub struct Masking
{
    #[cfg(feature = "async")]
    pub inner: Arc<BytesBlockStore>,

    #[cfg(not(feature = "async"))]
    pub inner: BytesBlockStore,
}

impl Masking
{
    #[cfg(not(feature = "async"))]
    pub fn from_buffer<R>(
        mut in_buf: &mut R,
        starting_pos: u64,
    ) -> Result<Self, String>
    where
        R: Read + Seek + Send + Sync + BufRead,
    {
        let inner = BytesBlockStore::from_buffer(&mut in_buf, starting_pos)?;
        Ok(Masking { inner })
    }

    #[cfg(feature = "async")]
    pub async fn from_buffer(
        in_buf: &mut tokio::io::BufReader<tokio::fs::File>,
        filename: String,
        starting_pos: u64,
    ) -> Result<Self, String>
    {
        let inner =
            BytesBlockStore::from_buffer(in_buf, filename, starting_pos)
                .await?;

        Ok(Masking {
            inner: Arc::new(inner),
        })
    }

    #[cfg(feature = "async")]
    pub async fn stream(
        self: Arc<Self>,
        fhm: Arc<AsyncFileHandleManager>,
    ) -> impl Stream<Item = (u32, bytes::Bytes)>
    {
        Arc::clone(&self.inner).stream(fhm).await

        // let bincode_config =
        // bincode::config::standard().with_variable_int_encoding();

        // Arc::clone(&self.inner).stream(fhm).await.map(move |x| {
        // log::trace!("Size of mask (still RLE): {}", x.1.len());
        // let mask: Vec<(u64, u8)> =
        // bincode::decode_from_slice(&x.1, bincode_config)
        // .expect("Failed to decode mask")
        // .0;
        // (x.0, bytes::Bytes::from(rle_decode(&mask)))
        // })
    }

    #[cfg(not(feature = "async"))]
    /// Masks the sequence in place
    pub fn mask_sequence<R>(
        &mut self,
        in_buf: &mut R,
        loc: &[Loc],
        seq: &mut [u8],
    ) where
        R: Read + Seek + Send + Sync,
    {
        let arch = Arch::new();

        let mask_raw = self.inner.get(in_buf, loc);

        let bincode_config =
            bincode::config::standard().with_variable_int_encoding();

        let mask_raw: Vec<u8> =
            bincode::decode_from_slice(&mask_raw, bincode_config)
                .expect("Failed to decode mask")
                .0;

        let mask_raw = rle_decode(&mask_raw);

        arch.dispatch(|| {
            for (i, m) in mask_raw.iter().enumerate() {
                seq[i] = if *m {
                    seq[i].to_ascii_lowercase()
                } else {
                    seq[i]
                };
            }
        })
    }

    #[cfg(feature = "async")]
    /// preload the masking data from disk
    pub async fn get_mask(
        &self,
        in_buf: &mut tokio::sync::OwnedMutexGuard<BufReader<File>>,
        loc: &[Loc],
    ) -> bytes::Bytes
    {
        let mask = self.inner.get(in_buf, loc).await;

        // let bincode_config =
        // bincode::config::standard().with_variable_int_encoding();
        //
        // let mask: Vec<(u64, u8)> =
        // bincode::decode_from_slice(&mask, bincode_config)
        // .expect("Failed to decode mask")
        // .0;
        // bytes::Bytes::from(rle_decode(&mask))
        mask
    }
}

#[cfg(feature = "async")]
#[inline]
/// Masks the sequence in place
pub fn mask_sequence(seq: &mut [u8], mask_raw: bytes::Bytes)
{
    let arch = Arch::new();

    let config = bincode::config::standard().with_variable_int_encoding();

    // let mask: Vec<(u64, u8)> =
    // bincode::decode_from_slice(&mask_raw, config).unwrap().0;

    // will this work without bincode? dunno
    let mask: Vec<u8> = mask_raw.to_vec();

    let mask_raw = rle_decode(&mask);

    arch.dispatch(|| {
        for (i, m) in mask_raw.iter().enumerate() {
            seq[i] = if *m {
                seq[i].to_ascii_lowercase()
            } else {
                seq[i]
            };
        }
    });
}

#[cfg(feature = "async")]
pub struct MaskingBlockReader
{
    active: Option<Pin<Box<(dyn Stream<Item = (u32, Bytes)> + Send)>>>,
    cached_block: (u32, Bytes),
}

#[cfg(feature = "async")]
impl MaskingBlockReader
{
    pub async fn new(
        block_store: Arc<Masking>,
        fhm: Arc<AsyncFileHandleManager>,
    ) -> Self
    {
        let store = Arc::clone(&block_store);
        let stream = store.stream(fhm).await;

        let mut boxed = Box::pin(stream);
        let cached_block = boxed.next().await.unwrap();

        MaskingBlockReader {
            active: Some(boxed),
            cached_block,
        }
    }

    /// Hack for when no masking is found
    pub async fn dummy() -> Self
    {
        MaskingBlockReader {
            active: None,
            cached_block: (0, Bytes::new()),
        }
    }

    pub async fn next(&mut self, loc: &[Loc]) -> Option<Bytes>
    {
        let mut locs = &loc[..];
        let mut results = Vec::new();

        while !locs.is_empty() {
            let loc = &locs[0];
            let block = loc.block;

            debug_assert!(
                block >= self.cached_block.0,
                "Block: {} Cached: {}",
                block,
                self.cached_block.0
            );

            if block == self.cached_block.0 {
                let start = loc.start as usize;
                let end = (loc.start + loc.len) as usize;
                let j = self.cached_block.1.slice(start..end);
                results.push(j);
                locs = &locs[1..];
            } else {
                let block = self.active.as_mut().unwrap().next().await.unwrap();
                self.cached_block = block;
            }
        }

        if results.is_empty() {
            return None;
        }

        let mut result = BytesMut::new();
        for r in results {
            result.put(r);
        }

        Some(result.freeze())
    }
}

#[cfg(test)]
mod tests
{
    use super::*;
    use rand::Rng;
    use std::{
        io::Cursor,
        sync::{Arc, Mutex},
    };

    #[test]
    fn test_masking_basics()
    {
        let seq = b"actgACTG";
        let value: Vec<bool> = seq.iter().map(|x| x >= &b'Z').collect();
        assert!(
            value == vec![true, true, true, true, false, false, false, false]
        );
    }

    #[cfg(not(feature = "async"))]
    #[test]
    fn test_masking()
    {
        let mut buffer = vec![0x0];
        buffer.reserve(64 * 1024);
        let mut output_buffer = Arc::new(std::sync::Mutex::new(Box::new(
            std::io::Cursor::new(buffer),
        )));

        // Move cursor to 1
        output_buffer
            .lock()
            .unwrap()
            .seek(std::io::SeekFrom::Start(1))
            .unwrap();

        let mut output_worker = crate::io::worker::Worker::new(output_buffer)
            .with_buffer_size(1024);
        output_worker.start();

        let output_queue = output_worker.get_queue();

        let mut compression_workers = CompressionWorker::new()
            .with_buffer_size(16)
            .with_threads(1_u16)
            .with_output_queue(Arc::clone(&output_queue));

        compression_workers.start();
        let mut compression_workers = Arc::new(compression_workers);

        let mut masking = MaskingStoreBuilder::default()
            .with_compression_worker(Arc::clone(&compression_workers));
        let test_seqs = vec![
            "ATCGGGGCAACTACTACGATCAcccccccccaccatgcacatcatctacAAAActcgacaAcatcgacgactacgaa",
            "aaaaaaaaaaaaTACTACGATCAcccccccccaccatgcacatcatctacAAAActcgacaAcatcgacgactACGA",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAaaaaaaaaaaaaaAAAAAAAAAAAAAAAAA",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaTAa",
        ];

        let seq = test_seqs[0].as_bytes();
        let loc = masking.add(seq.to_vec());

        for _ in 0..1000 {
            (0..test_seqs.len()).for_each(|i| {
                let seq = test_seqs[i].as_bytes();
                masking.add(seq.to_vec());
            });
        }

        let seq = test_seqs[0].as_bytes();
        let loc2 = masking.add(seq.to_vec());

        let seq = test_seqs[3].as_bytes();
        let loc3 = masking.add(seq.to_vec());
        assert!(loc3 != vec![]);

        for _ in 0..1000 {
            (0..test_seqs.len()).for_each(|i| {
                let seq = test_seqs[i].as_bytes();
                masking.add(seq.to_vec());
            });
        }

        // compression_workers.shutdown();
        println!("Finalizing masking store...");
        masking
            .finalize()
            .expect("Failed to finalize masking store");
        std::thread::sleep(std::time::Duration::from_millis(500));

        println!("Masking store finalized, now doing output worker...");
        output_worker.shutdown();

        // todo bad hack!
        std::thread::sleep(std::time::Duration::from_millis(500));

        let mut output_buffer = output_worker.into_inner();

        masking
            .write_block_locations(&mut output_buffer)
            .expect("Failed to write block locations");
        let pos = output_buffer.stream_position().unwrap();
        masking.write_header(pos, &mut output_buffer);

        println!("Got it opening now");

        let mut masking =
            Masking::from_buffer(&mut output_buffer, pos).unwrap();

        let mut seq = test_seqs[0].as_bytes().to_vec();
        masking.mask_sequence(&mut output_buffer, &loc, &mut seq);
        // Print out as text
        println!("{}", std::str::from_utf8(&seq).unwrap());
        assert!(seq == test_seqs[0].as_bytes());
    }
}
