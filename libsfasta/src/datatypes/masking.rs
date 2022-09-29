// TODO: Just store masking as Vec<Commands> and ignore u32 and benchmark
// TODO: And while benchmarking, go ahead and store as bits of 0's and 1's for entire seq length
// and compare...

use std::io::{Read, Seek, SeekFrom, Write};

use crate::datatypes::zstd_encoder;
use crate::masking::ml32bit::*;
use crate::*;

use bitpacking::{BitPacker, BitPacker8x};
use bumpalo::Bump;

#[derive(Debug, Clone, bincode::Encode, bincode::Decode, Eq, PartialEq)]
pub enum MaskingStyle {
    Ml32bit,
    Binary,
}

pub struct Masking {
    location: u64,
    bitpack_len: u64,
    data: Option<Vec<u32>>, // Only stored for writing
    data_binary: Option<Vec<bool>>, // Only stored for writing   
    num_bits: u8,
    cache: Option<(u32, Vec<u32>)>,
    total_blocks: u32,
    bump: Option<Bump>,
    pub style: MaskingStyle,
}

impl Default for Masking {
    fn default() -> Self {
        Masking {
            location: 0,
            bitpack_len: 0,
            data: None,
            data_binary: None,
            num_bits: 0,
            cache: None,
            total_blocks: 0,
            bump: None,
            style: MaskingStyle::Binary,
        }
    }
}

impl Masking {
    pub fn add_masking(&mut self, seq: &[u8]) -> Option<(u32, u32)> {
        if self.bump.is_none() {
            self.bump = Some(Bump::new());
        }

        let bump = self.bump.as_mut().unwrap();

        // Start and LENGTH (not end)
        if self.style == MaskingStyle::Ml32bit && self.data.is_none() {
            self.data = Some(Vec::with_capacity(512 * 1024));
        }

        if self.style == MaskingStyle::Binary && self.data_binary.is_none() {
            self.data_binary = Some(Vec::with_capacity(2 * 1024 * 1024));
        }

        // Are any lowercase?
        if !seq.iter().any(|x| x.is_ascii_lowercase()) {
            return None;
        }

        let start: usize; 
        let len: usize;

        if self.style == MaskingStyle::Ml32bit {

            let data = self.data.as_mut().unwrap();

            let ranges = bump.alloc(get_masking_ranges(seq));
            let ml32bit = bump.alloc(convert_ranges_to_ml32bit(ranges));
            let ml32bit = bump.alloc(pad_commands_to_u32(ml32bit));
            let commands = bump.alloc(convert_commands_to_u32(ml32bit));

            len = commands.len();
            start = data.len();
            data.extend(commands.iter());

            bump.reset();
        } else {
            let data = self.data_binary.as_mut().unwrap();
            start = data.len();
            data.extend(seq.iter().map(|x| x.is_ascii_lowercase()));
            len = seq.len();
        }

        Some((start as u32, len as u32))
    }

    pub fn write_to_buffer<W>(&mut self, mut out_buf: &mut W) -> Option<u64>
    where
        W: Write + Seek,
    {
        // self.data.as_ref()?;
        // self.data = Some(Vec::new());

        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        self.location = out_buf.seek(SeekFrom::Current(0)).unwrap();

        // let (num_bits, packed) = bitpack_u32(self.data.as_ref().unwrap());

        // let mut bitpacked_len: u64 = 0;

        bincode::encode_into_std_write(&self.style, &mut out_buf, bincode_config).unwrap();
        // bincode::encode_into_std_write(&self.bitpack_len, &mut out_buf, bincode_config).unwrap();
        // bincode::encode_into_std_write(&num_bits, &mut out_buf, bincode_config).unwrap();
        bincode::encode_into_std_write(&self.total_blocks, &mut out_buf, bincode_config).unwrap();

        /*if self.style == MaskingStyle::Ml32bit {
            for bp in packed {
                self.total_blocks = self.total_blocks.saturating_add(1);
                let len = bincode::encode_into_std_write(&bp, &mut out_buf, bincode_config).unwrap();
                if bitpacked_len == 0 && bp.is_packed() {
                    bitpacked_len = len as u64;
                } else if bp.is_packed() {
                    assert_eq!(bitpacked_len, len as u64);
                }
            }
        } else {*/
            let mut encoder = zstd_encoder(3, None);
            let mut cseq: Vec<u8> = Vec::with_capacity(2 * 1024 * 1024);
            let mut uncompressed = Vec::with_capacity(2 * 1024 * 1024);
            for chunk in self.data_binary.take().unwrap().chunks(512 * 1024) {
                bincode::encode_into_std_write(chunk.to_vec(), &mut uncompressed, bincode_config)
                    .unwrap();
                encoder
                    .compress_to_buffer(&uncompressed, &mut cseq)
                    .unwrap();
                bincode::encode_into_std_write(&cseq, &mut out_buf, bincode_config).unwrap();
                cseq.clear();
                uncompressed.clear();
            }
        //}

        let end = out_buf.seek(SeekFrom::Current(0)).unwrap();
        out_buf.seek(SeekFrom::Start(self.location)).unwrap();

        bincode::encode_into_std_write(&self.style, &mut out_buf, bincode_config).unwrap();
        // bincode::encode_into_std_write(&bitpacked_len, &mut out_buf, bincode_config).unwrap();
        // bincode::encode_into_std_write(&num_bits, &mut out_buf, bincode_config).unwrap();
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
            let mut encoder = zstd_encoder(1, None);
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

    pub fn from_buffer<R>(mut in_buf: &mut R, starting_pos: u64) -> Result<Self, String>
    where
        R: Read + Seek,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        let mut masking = Masking::default();

        in_buf.seek(SeekFrom::Start(starting_pos)).unwrap();
        masking.location = starting_pos;

        masking.style = match bincode::decode_from_std_read(&mut in_buf, bincode_config) {
            Ok(x) => x,
            Err(e) => return Err(format!("Error reading masking style: {}", e)),
        };
        masking.bitpack_len = match bincode::decode_from_std_read(&mut in_buf, bincode_config) {
            Ok(x) => x,
            Err(e) => return Err(format!("Error decoding bitpack_len: {}", e)),
        };
        masking.num_bits = match bincode::decode_from_std_read(&mut in_buf, bincode_config) {
            Ok(x) => x,
            Err(e) => return Err(format!("Error decoding num_bits: {}", e)),
        };
        masking.total_blocks = match bincode::decode_from_std_read(&mut in_buf, bincode_config) {
            Ok(x) => x,
            Err(e) => return Err(format!("Error decoding total_blocks: {}", e)),
        };

        Ok(masking)
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
        bp.unpack(self.num_bits).unwrap()
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
        let mut masking = Masking::from_buffer(&mut buffer, 0).unwrap();

        let mut seq = test_seqs[0].as_bytes().to_ascii_uppercase();
        masking.mask_sequence(&mut buffer, loc, &mut seq);
        assert_eq!(seq, test_seqs[0].as_bytes());

        let mut seq = test_seqs[0].as_bytes().to_ascii_uppercase();
        masking.mask_sequence(&mut buffer, loc2, &mut seq);
        assert_eq!(seq, test_seqs[0].as_bytes());
    }
}
