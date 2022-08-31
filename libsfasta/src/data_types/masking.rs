use std::io::{Read, Seek, SeekFrom, Write};

use crate::data_types::{zstd_encoder, CompressionType, Loc};
use crate::masking::ml32bit::*;
use crate::*;

use bitpacking::{BitPacker, BitPacker8x};

pub struct Masking {
    location: u64,
    bitpack_len: u64,
    pub data: Option<Vec<u32>>, // Only stored for writing
    num_bits: u8,
    cache: Option<(u32, Vec<u32>)>,
    total_blocks: u32,
}

impl Default for Masking {
    fn default() -> Self {
        Masking {
            location: 0,
            bitpack_len: 0,
            data: None,
            num_bits: 0,
            cache: None,
            total_blocks: 0,
        }
    }
}

impl Masking {
    pub fn add_masking(&mut self, seq: &[u8]) -> Option<(u32, u32)> {
        // Start and LENGTH (not end)
        if self.data.is_none() {
            self.data = Some(Vec::new());
        }

        // Is any lowercase?
        if !seq.iter().any(|x| x.is_ascii_lowercase()) {
            return None;
        }

        let data = self.data.as_mut().unwrap();
        // BitPacker8x::BLOCK_LEN

        let commands = convert_commands_to_u32(&pad_commands_to_u32(&convert_ranges_to_ml32bit(
            &get_masking_ranges(seq),
        )));

        let len = commands.len();
        let start = data.len();
        data.extend(commands);

        Some((start as u32, len as u32))
    }

    pub fn write_to_buffer<W>(&mut self, mut out_buf: &mut W) -> Option<u64>
    where
        W: Write + Seek,
    {
        self.data.as_ref()?;

        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        self.location = out_buf.seek(SeekFrom::Current(0)).unwrap();
        let (num_bits, packed) = bitpack_u32(&self.data.as_ref().unwrap());

        let mut bitpacked_len: u64 = 0;

        bincode::encode_into_std_write(&self.bitpack_len, &mut out_buf, bincode_config).unwrap();
        bincode::encode_into_std_write(&num_bits, &mut out_buf, bincode_config).unwrap();
        bincode::encode_into_std_write(&self.total_blocks, &mut out_buf, bincode_config).unwrap();

        for bp in packed {
            self.total_blocks = self.total_blocks.saturating_add(1);
            let len = bincode::encode_into_std_write(&bp, &mut out_buf, bincode_config).unwrap();
            if bitpacked_len == 0 && bp.is_packed() {
                bitpacked_len = len as u64;
            } else if bp.is_packed() {
                assert_eq!(bitpacked_len, len as u64);
            }
        }

        let end = out_buf.seek(SeekFrom::Current(0)).unwrap();
        out_buf.seek(SeekFrom::Start(self.location)).unwrap();

        bincode::encode_into_std_write(&bitpacked_len, &mut out_buf, bincode_config).unwrap();
        bincode::encode_into_std_write(&num_bits, &mut out_buf, bincode_config).unwrap();
        bincode::encode_into_std_write(&self.total_blocks, &mut out_buf, bincode_config).unwrap();

        // Back to the end so we don't interfere with anything...
        out_buf.seek(SeekFrom::Start(end)).unwrap();

        Some(self.location)
    }

    pub fn write_to_buffer_zstd<W>(&mut self, mut out_buf: &mut W) -> Option<u64>
    where
        W: Write + Seek,
    {
        self.data.as_ref()?;

        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        self.location = out_buf.seek(SeekFrom::Current(0)).unwrap();

        for chunk in self.data.take().unwrap().chunks(2 * 1024) {
            let mut encoder = zstd_encoder(1);
            let mut cseq: Vec<u8> = Vec::with_capacity(2 * 1024 * 32);
            let mut uncompressed = Vec::with_capacity(2 * 1024 * 32);
            bincode::encode_into_std_write(chunk.to_vec(), &mut uncompressed, bincode_config)
                .unwrap();
            encoder
                .compress_to_buffer(&uncompressed, &mut cseq)
                .unwrap();
            bincode::encode_into_std_write(&cseq, &mut out_buf, bincode_config).unwrap();
        }

        let end = out_buf.seek(SeekFrom::Current(0)).unwrap();
        out_buf.seek(SeekFrom::Start(self.location)).unwrap();

        // Back to the end so we don't interfere with anything...
        out_buf.seek(SeekFrom::Start(end)).unwrap();

        Some(self.location)
    }

    pub fn from_buffer<R>(mut in_buf: &mut R, starting_pos: u64) -> Self
    where
        R: Read + Seek,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        let mut masking = Masking::default();

        in_buf.seek(SeekFrom::Start(starting_pos)).unwrap();
        masking.location = starting_pos;

        masking.bitpack_len = bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();
        masking.num_bits = bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();
        masking.total_blocks = bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();

        masking
    }

    pub fn prefetch<R>(&mut self, in_buf: &mut R)
    where
        R: Read + Seek,
    {
        let mut data = Vec::new();
        for i in 0..self.total_blocks {
            data.extend(self.get_block_uncached(in_buf, i as u32));
        }
    }

    pub fn get_block<R>(&mut self, in_buf: &mut R, block: u32) -> &[u32]
    where
        R: Read + Seek,
    {
        if self.cache.is_some() && self.cache.as_ref().unwrap().0 == block {
            &self.cache.as_ref().unwrap().1
        } else {
            let blockdata = self.get_block_uncached(in_buf, block);
            self.cache = Some((block, blockdata));
            &self.cache.as_ref().unwrap().1
        }
    }

    pub fn get_block_uncached<R>(&mut self, mut in_buf: &mut R, block: u32) -> Vec<u32>
    where
        R: Read + Seek,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        let block_position = self.location + 8 + 1 + 4 + (block as u64 * self.bitpack_len);
        in_buf.seek(SeekFrom::Start(block_position)).unwrap();
        let bp: Packed = bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();
        bp.unpack(self.num_bits)
    }

    /// Masks the sequence in place
    pub fn mask_sequence<R>(&mut self, mut in_buf: &mut R, loc: (u32, u32), seq: &mut [u8])
    where
        R: Read + Seek,
    {
        let mut u32s: Vec<u32> = Vec::new();

        if self.data.is_some() {
            u32s.extend(
                self.data.as_ref().unwrap()[loc.0 as usize..(loc.0 + loc.1) as usize].iter(),
            );
        } else {
            let starting_block = loc.0 / BitPacker8x::BLOCK_LEN as u32;
            let ending_block = (loc.0 + loc.1 - 1) / BitPacker8x::BLOCK_LEN as u32;
            let mut in_block_pos = loc.0 % BitPacker8x::BLOCK_LEN as u32;

            let mut to_fetch = loc.1;

            for block in starting_block..=ending_block {
                assert!(to_fetch > 0);
                let unpacked = self.get_block(&mut in_buf, block);
                let mut end = std::cmp::min(BitPacker8x::BLOCK_LEN, to_fetch as usize);
                end = std::cmp::min(end, unpacked.len() - in_block_pos as usize);

                u32s.extend(&unpacked[in_block_pos as usize..in_block_pos as usize + end]);
                to_fetch = to_fetch.saturating_sub(end as u32);
                in_block_pos = 0;
            }
        }

        let commands = convert_u32_to_commands(&u32s);
        mask_sequence(&commands, seq);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_masking() {
        let mut masking = Masking::default();
        let test_seqs = vec![
            "ATCGGGGCAACTACTACGATCAcccccccccaccatgcacatcatctacAAAActcgacaAcatcgacgactacgaa",
            "aaaaaaaaaaaaTACTACGATCAcccccccccaccatgcacatcatctacAAAActcgacaAcatcgacgactACGA",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAaaaaaaaaaaaaaAAAAAAAAAAAAAAAAA",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ];

        let seq = test_seqs[0].as_bytes();
        let loc = masking.add_masking(seq).unwrap();

        for _ in 0..1000 {
            for i in 0..test_seqs.len() {
                let seq = test_seqs[i].as_bytes();
                masking.add_masking(seq);
            }
        }

        let seq = test_seqs[0].as_bytes();
        let loc2 = masking.add_masking(seq).unwrap();

        for _ in 0..1000 {
            for i in 0..test_seqs.len() {
                let seq = test_seqs[i].as_bytes();
                masking.add_masking(seq);
            }
        }

        let mut buffer = Cursor::new(Vec::new());
        masking.write_to_buffer(&mut buffer).unwrap();
        let mut masking = Masking::from_buffer(&mut buffer, 0);

        let mut seq = test_seqs[0].as_bytes().to_ascii_uppercase();
        masking.mask_sequence(&mut buffer, loc, &mut seq);
        assert_eq!(seq, test_seqs[0].as_bytes());

        let mut seq = test_seqs[0].as_bytes().to_ascii_uppercase();
        masking.mask_sequence(&mut buffer, loc2, &mut seq);
        assert_eq!(seq, test_seqs[0].as_bytes());
    }
}
