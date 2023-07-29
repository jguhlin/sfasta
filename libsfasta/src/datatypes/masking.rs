// NOTE: I've spent lots of time converting this to bitvec so that bool would be 1bit instead of 8bits (1 bytes)
// Compressed, this saves < 1Mbp on a 2.3Gbp uncompressed FASTA file... and triple the length for masking.
use std::io::{Read, Seek, Write};

use crate::datatypes::{BytesBlockStore, Loc};

use pulp::Arch;

#[derive(Clone)]
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
        // If none are lowercase, nope out here... Written in a way that allows for easy vectorization for SIMD
        let arch = Arch::new();

        // Significant speedup (70% reduction in total masking time)
        if arch.dispatch(|| {
            for x in seq.iter() {
                if x < &b'a' {
                    return true
                }
            }
            false
        }) {
            return None;
        }

        // No benefit to using pulp here... (even with for loop)
        let masked: Vec<u8> = seq.iter().map(|x| x > &b'Z').map(|x| x as u8).collect();
        
        Some(self.inner.add(&masked))
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

    /// Masks the sequence in place
    pub fn mask_sequence<R>(&mut self, in_buf: &mut R, loc: &[Loc], seq: &mut [u8])
    where
        R: Read + Seek,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_masking_basics() {
        let seq = b"actgACTG";
        let value: Vec<bool> = seq.iter().map(|x| x >= &b'Z').collect();
        assert!(value == vec![true, true, true, true, false, false, false, false]);
    }

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
        println!("{loc:#?}");

        for _ in 0..1000 {
            (0..test_seqs.len()).for_each(|i| {
                let seq = test_seqs[i].as_bytes();
                masking.add_masking(seq);
            });
        }

        let seq = test_seqs[0].as_bytes();
        let loc2 = masking.add_masking(seq).unwrap();
        println!("{loc2:#?}");

        let seq = test_seqs[3].as_bytes();
        let loc3 = masking.add_masking(seq).unwrap();

        for _ in 0..1000 {
            (0..test_seqs.len()).for_each(|i| {
                let seq = test_seqs[i].as_bytes();
                masking.add_masking(seq);
            });
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
