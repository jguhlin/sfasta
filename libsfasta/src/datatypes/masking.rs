use std::io::{Read, Seek, SeekFrom, Write};

use crate::datatypes::{BytesBlockStore, Loc};
use crate::*;

use bitpacking::{BitPacker, BitPacker8x};
use bitvec::prelude::*;
use bumpalo::Bump;
use pulp::Arch;

pub struct Masking {
    inner: BytesBlockStore,
}

impl Default for Masking {
    fn default() -> Self {
        Masking {
            inner: BytesBlockStore::default().with_block_size(512 * 1024),
        }
    }
}

impl Masking {
    pub fn with_block_size(mut self, block_size: usize) -> Self {
        self.inner = self.inner.with_block_size(block_size);
        self
    }

    pub fn add_masking(&mut self, seq: &[u8]) -> Option<Vec<Loc>> {
        // If none are lowercase, nope out here...
        let arch = Arch::new();

        if !seq.iter().any(|x| x.is_ascii_lowercase()) {
            return None;
        }

        let mut masked: BitVec<u64, Lsb0> =
            BitVec::from_iter(seq.iter().map(|x| x.is_ascii_lowercase()));

        // Convert from bitvec to u8
        let mut bytes = Vec::with_capacity(masked.len() / 8);
        masked.force_align();

        let mut chunks = masked.chunks_exact(8);
        let remainder = chunks.remainder();
        for chunk in chunks {
            bytes.push(chunk.load_le::<u8>());
        }

        if !remainder.is_empty() {
            bytes.push(remainder.load_le::<u8>());
        }

        Some(self.inner.add(&bytes))
    }

    pub fn write_to_buffer<W>(&mut self, mut out_buf: &mut W) -> Option<u64>
    where
        W: Write + Seek,
    {
        self.inner.write_to_buffer(&mut out_buf)
    }

    pub fn from_buffer<R>(mut in_buf: &mut R, starting_pos: u64) -> Result<Self, String>
    where
        R: Read + Seek,
    {
        let inner = BytesBlockStore::from_buffer(&mut in_buf, starting_pos)?;
        Ok(Masking { inner })
    }

    pub fn prefetch<R>(&mut self, in_buf: &mut R)
    where
        R: Read + Seek,
    {
        self.inner.prefetch(in_buf)
    }

    pub fn get_block<R>(&mut self, in_buf: &mut R, block: u32) -> BitVec<u8, Lsb0>
    where
        R: Read + Seek,
    {
        let block = self.inner.get_block(in_buf, block);
        BitVec::from_vec(block)
    }

    /// Masks the sequence in place
    pub fn mask_sequence<R>(&mut self, in_buf: &mut R, loc: &[Loc], seq: &mut [u8])
    where
        R: Read + Seek,
    {
        let mut mask = self.inner.get(in_buf, &loc);
        let mut mask: BitVec<u8, Lsb0> = BitVec::from_vec(mask);

        for (i, m) in mask.drain(..).enumerate() {
            if m {
                seq[i] = seq[i].to_ascii_lowercase();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_masking() {
        use simdutf8::basic::from_utf8;

        let mut masking = Masking::default();
        let test_seqs = vec![
            "ATCGGGGCAACTACTACGATCAcccccccccaccatgcacatcatctacAAAActcgacaAcatcgacgactacgaa",
            "aaaaaaaaaaaaTACTACGATCAcccccccccaccatgcacatcatctacAAAActcgacaAcatcgacgactACGA",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAaaaaaaaaaaaaaAAAAAAAAAAAAAAAAA",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaTAa",
        ];

        let seq = test_seqs[0].as_bytes();
        println!("{:#?}", test_seqs.len());
        let loc = masking.add_masking(seq).unwrap();
        println!("{:#?}", loc);

        for _ in 0..1000 {
            for i in 0..test_seqs.len() {
                let seq = test_seqs[i].as_bytes();
                masking.add_masking(seq);
            }
        }

        let seq = test_seqs[0].as_bytes();
        let loc2 = masking.add_masking(seq).unwrap();
        println!("{:#?}", loc2);

        let seq = test_seqs[3].as_bytes();
        let loc3 = masking.add_masking(seq).unwrap();

        for _ in 0..1000 {
            for i in 0..test_seqs.len() {
                let seq = test_seqs[i].as_bytes();
                masking.add_masking(seq);
            }
        }

        let mut buffer = Cursor::new(Vec::new());
        masking.write_to_buffer(&mut buffer).unwrap();
        let mut masking = Masking::from_buffer(&mut buffer, 0).unwrap();

        let mut seq = test_seqs[0].as_bytes().to_ascii_uppercase();
        masking.mask_sequence(&mut buffer, &loc, &mut seq);
        assert_eq!(seq, test_seqs[0].as_bytes());

        let mut seq = test_seqs[0].as_bytes().to_ascii_uppercase();
        masking.mask_sequence(&mut buffer, &loc2, &mut seq);
        assert_eq!(seq, test_seqs[0].as_bytes());

        let mut seq = test_seqs[3].as_bytes().to_ascii_uppercase();
        masking.mask_sequence(&mut buffer, &loc3, &mut seq);
        assert_eq!(seq, test_seqs[3].as_bytes());
        println!("{:#?}", from_utf8(&seq));
    }
}
