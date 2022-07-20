use std::convert::TryFrom;
use std::io::{Read, Seek, SeekFrom, Write};

use bitpacking::{BitPacker, BitPacker8x};

use crate::utils::{bitpack_u32, Bitpacked};

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

    pub fn bitpack(&self) -> Vec<Bitpacked> {
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

    pub fn write_to_buffer<W>(&mut self, mut out_buf: &mut W)
    where
        W: Write + Seek,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        // Write the locs_start
        let bitpacked = self.bitpack();
        bincode::encode_into_std_write(&self.locs_start, &mut out_buf, bincode_config)
            .expect("Bincode error");

        let blocks_locs_loc_loc = out_buf.seek(SeekFrom::Current(0)).unwrap();

        // Write the block_locs
        bincode::encode_into_std_write(&self.block_locs, &mut out_buf, bincode_config)
            .expect("Bincode error"); // this one is a dummy value

        // Write the bitpacked data
        for bp in bitpacked {
            self.block_locs
                .push(out_buf.seek(SeekFrom::Current(0)).unwrap());
            bincode::encode_into_std_write(bp, &mut out_buf, bincode_config)
                .expect("Bincode error");
        }

        self.blocks_locs_loc = out_buf.seek(SeekFrom::Current(0)).unwrap();

        // Output the blocks locs loc
        bincode::encode_into_std_write(&self.blocks_locs_loc, &mut out_buf, bincode_config)
            .expect("Bincode error");

        let end = out_buf.seek(SeekFrom::Current(0)).unwrap();
        out_buf.seek(SeekFrom::Start(blocks_locs_loc_loc)).unwrap();

        // Output the the end
        bincode::encode_into_std_write(&end, &mut out_buf, bincode_config).expect("Bincode error");

        // Go back to the end so we don't screw up other operations...
        out_buf.seek(SeekFrom::Start(end)).unwrap();
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

    #[test]
    pub fn test_dual_index() {
        let mut out_buf: Cursor<Vec<u8>> = Cursor::new(Vec::new());
        let mut di = DualIndex::new(0);

        for i in (0_u64..10000).step_by(2) {
            di.locs.push(i);
        }

        di.write_to_buffer(&mut out_buf);

        // println!("{:#?}", out_buf.into_inner);

        // let mut in_buf: Cursor<Vec<u8>> = Cursor::new(out_buf.into_inner());
        let mut in_buf = out_buf;
        let di2 = DualIndex::read_from_buffer(&mut in_buf);
        assert_eq!(di.locs_start, di2.locs_start);
        assert_eq!(di.block_locs, di2.block_locs);
        assert_eq!(di.blocks_locs_loc, di2.blocks_locs_loc);
        assert_eq!(di.find_loc(&mut in_buf, 1000), 2000);
    }
}
