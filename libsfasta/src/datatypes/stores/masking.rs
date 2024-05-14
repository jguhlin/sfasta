// NOTE: I've spent lots of time converting this to bitvec so that
// bool would be 1bit instead of 8bits (1 bytes) Compressed, this
// saves < 1Mbp on a 2.3Gbp uncompressed FASTA file... and triple the
// length in time for masking. TODO: Try stream vbytes for this...
use std::{
    io::{BufRead, Read, Seek, Write},
    sync::Arc,
};

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
    inner: BytesBlockStoreBuilder,
}

impl Builder<Vec<u8>> for MaskingStoreBuilder
{
    fn add(&mut self, input: Vec<u8>) -> Result<Vec<Loc>, &str>
    {
        Ok(self.add(input))
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

    pub fn add(&mut self, seq: Vec<u8>) -> Vec<Loc>
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

        // No benefit to using pulp here... (even with for loop)
        let masked: Vec<u8> =
            seq.iter().map(|x| x > &b'`').map(|x| x as u8).collect();

        self.inner
            .add(masked)
            .expect("Failed to add masking to block store")
    }

    pub fn finalize(&mut self) -> Result<(), BlockStoreError>
    {
        self.inner.finalize()
    }
}

pub struct Masking
{
    inner: BytesBlockStore,
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
        starting_pos: u64,
    ) -> Result<Self, String>
    {
        let inner = BytesBlockStore::from_buffer(in_buf, starting_pos).await?;

        Ok(Masking { inner })
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

        arch.dispatch(|| {
            for (i, m) in mask_raw.iter().enumerate() {
                seq[i] = if *m == 1 {
                    seq[i].to_ascii_lowercase()
                } else {
                    seq[i]
                };
            }
        })
    }

    #[cfg(feature = "async")]
    /// Masks the sequence in place
    pub async fn mask_sequence(
        &self,
        in_buf: &mut OwnedRwLockWriteGuard<BufReader<File>>,
        loc: &[Loc],
        seq: &mut [u8],
    )
    {
        let arch = Arch::new();

        let mask_raw = self.inner.get(in_buf, loc).await;

        arch.dispatch(|| {
            for (i, m) in mask_raw.iter().enumerate() {
                seq[i] = if *m == 1 {
                    seq[i].to_ascii_lowercase()
                } else {
                    seq[i]
                };
            }
        });
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
