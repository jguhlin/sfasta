// TODO: Just store masking as Vec<Commands> and ignore u32 and benchmark
// TODO: And while benchmarking, go ahead and store as bits of 0's and 1's for entire seq length
// and compare...
// TODO: Support bitvec instead of Vec<bool> Urgent (bool is 1 byte, bitvec is 1 bit)

use std::io::{Read, Seek, SeekFrom, Write};

use crate::datatypes::zstd_encoder;
use crate::masking::ml32bit::*;
use crate::*;

use bitpacking::{BitPacker, BitPacker8x};
use bitvec::prelude::*;
use bumpalo::Bump;

static BLOCK_SIZE: usize = 512 * 1024;

#[derive(Debug, Clone, bincode::Encode, bincode::Decode, Eq, PartialEq)]
pub enum MaskingStyle {
    Ml32bit,
    Binary,
}

pub struct Masking {
    location: u64,
    index_location: u64,
    data: Option<Vec<u32>>,                 // Only stored for writing
    data_binary: Option<BitVec<u64, Lsb0>>, // Only stored for writing
    cache: Option<(u32, BitVec<u64, Lsb0>)>,
    total_blocks: u32,
    bump: Option<Bump>,
    pub style: MaskingStyle,
    index: Option<Vec<u64>>,
}

impl Default for Masking {
    fn default() -> Self {
        Masking {
            location: 0,
            index_location: 0,
            data: None,
            data_binary: None,
            cache: None,
            total_blocks: 0,
            bump: None,
            style: MaskingStyle::Binary,
            index: None,
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
            self.data_binary = Some(BitVec::with_capacity(2 * 1024 * 1024));
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
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        self.location = out_buf.seek(SeekFrom::Current(0)).unwrap();

        bincode::encode_into_std_write(&self.style, &mut out_buf, bincode_config).unwrap();
        bincode::encode_into_std_write(&self.total_blocks, &mut out_buf, bincode_config).unwrap();
        bincode::encode_into_std_write(&self.index_location, &mut out_buf, bincode_config).unwrap();

        let mut index: Vec<u64> = Vec::with_capacity(self.total_blocks as usize);

        let mut encoder = zstd_encoder(3, None);
        let mut cseq: Vec<u8> = Vec::with_capacity(2 * 1024 * 1024);
        let mut uncompressed = Vec::with_capacity(2 * 1024 * 1024);
        for (i, chunk) in self
            .data_binary
            .take()
            .unwrap()
            .chunks(BLOCK_SIZE)
            .enumerate()
        {
            index.push(out_buf.seek(SeekFrom::Current(0)).unwrap());
            let to_encode: Vec<u32> = chunk.into_iter().map(|x| *x as u32).collect();
            bincode::encode_into_std_write(to_encode, &mut uncompressed, bincode_config).unwrap();
            encoder
                .compress_to_buffer(&uncompressed, &mut cseq)
                .unwrap();
            bincode::encode_into_std_write(&cseq, &mut out_buf, bincode_config).unwrap();
            cseq.clear();
            uncompressed.clear();
        }

        self.index_location = out_buf.seek(SeekFrom::Current(0)).unwrap();
        bincode::encode_into_std_write(index, &mut uncompressed, bincode_config).unwrap();
        encoder
            .compress_to_buffer(&uncompressed, &mut cseq)
            .unwrap();
        bincode::encode_into_std_write(&cseq, &mut out_buf, bincode_config).unwrap();

        let end = out_buf.seek(SeekFrom::Current(0)).unwrap();
        out_buf.seek(SeekFrom::Start(self.location)).unwrap();

        bincode::encode_into_std_write(&self.style, &mut out_buf, bincode_config).unwrap();
        bincode::encode_into_std_write(&self.total_blocks, &mut out_buf, bincode_config).unwrap();
        bincode::encode_into_std_write(&self.index_location, &mut out_buf, bincode_config).unwrap();

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

        (masking.style, masking.total_blocks, masking.index_location) =
            bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();

        let mut cseq: Vec<u8> = Vec::with_capacity(2 * 1024 * 1024);
        let mut uncompressed: Vec<u8> = Vec::with_capacity(2 * 1024 * 1024);
        in_buf
            .seek(SeekFrom::Start(masking.index_location))
            .unwrap();
        let index_compressed: Vec<u8> =
            bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();
        let mut zstd_decompressor = zstd::bulk::Decompressor::new().unwrap();
        zstd_decompressor.include_magicbytes(false).unwrap();
        zstd_decompressor
            .decompress_to_buffer(&index_compressed, &mut uncompressed)
            .unwrap();
        let index: Vec<u64> =
            bincode::decode_from_std_read(&mut uncompressed.as_slice(), bincode_config).unwrap();
        masking.index = Some(index);

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

    pub fn get_block<R>(&mut self, in_buf: &mut R, block: u32) -> &BitSlice<u64, Lsb0>
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

    pub fn get_block_uncached<R>(&mut self, mut in_buf: &mut R, block: u32) -> BitVec<u64, Lsb0>
    where
        R: Read + Seek,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        let block_position = self.index.as_ref().unwrap()[block as usize];
        in_buf.seek(SeekFrom::Start(block_position)).unwrap();
        let compressed: Vec<u8> =
            bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();
        let mut uncompressed: Vec<u8> = Vec::with_capacity(4 * 1024 * 1024);
        let mut zstd_decompressor = zstd::bulk::Decompressor::new().unwrap();
        zstd_decompressor.include_magicbytes(false).unwrap();
        zstd_decompressor
            .decompress_to_buffer(&compressed, &mut uncompressed)
            .unwrap();
        let blockdata: Vec<u32> =
            bincode::decode_from_std_read(&mut uncompressed.as_slice(), bincode_config).unwrap();
        // Convert back to bitvec
        let blockdata: Vec<u64> = blockdata.into_iter().map(|x| x as u64).collect();
        BitVec::from_vec(blockdata)
    }

    /// Masks the sequence in place
    pub fn mask_sequence<R>(&mut self, in_buf: &mut R, loc: (u32, u32), seq: &mut [u8])
    where
        R: Read + Seek,
    {
        let starting_block = loc.0 as usize / BLOCK_SIZE;
        let ending_block = (loc.0 + loc.1) as usize / BLOCK_SIZE;
        let mut in_block_pos = loc.0 as usize % BLOCK_SIZE;

        let mut to_fetch = loc.1;

        // TODO: Not working!
        for block in starting_block..=ending_block {
            let blockdata = self.get_block(in_buf, block as u32);
            let mut block_pos = in_block_pos;

            while block_pos < BLOCK_SIZE && to_fetch > 0 {
                if blockdata[block_pos] {
                    // Make matching position of sequence lowercase
                    seq[loc.0 as usize
                        + (block_pos - in_block_pos)
                        // + (ending_block - block) * BLOCK_SIZE]
                    ]
                        .make_ascii_lowercase();
                }
                block_pos += 1;
                to_fetch -= 1;
            }

            in_block_pos = 0;
        }

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
