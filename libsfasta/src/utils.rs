/// Opens files, including compressed files (gzip or snappy)
use std::path::Path;
use std::{cmp::min, fmt, io::Read};

use bitpacking::{BitPacker, BitPacker8x};
use crossbeam::channel;

pub fn create_dict(input: &[u8], block_size: usize) -> Vec<u8>
{
    let block_counts = input.len() / block_size;
    let input = &input[..block_counts * block_size];
    let block_sizes = vec![block_size; block_counts];
    zstd::dict::from_continuous(input, &block_sizes, block_size).unwrap()
}

// From: https://stackoverflow.com/questions/72571846/crossbeam-receiver-to-bufread
pub struct CrossbeamReader
{
    input: channel::Receiver<ReaderData>,
    buffer: Vec<u8>,
    offset: usize,
    closed: bool,
}

pub enum ReaderData
{
    Data(Vec<u8>),
    EOF,
}

impl CrossbeamReader
{
    pub fn from_channel(input: channel::Receiver<ReaderData>) -> CrossbeamReader
    {
        CrossbeamReader {
            input,
            buffer: Vec::with_capacity(2048),
            offset: 0,
            closed: false,
        }
    }
}

impl Read for CrossbeamReader
{
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize>
    {
        if self.closed {
            return Ok(0);
        }
        while self.offset >= self.buffer.len() {
            self.buffer = match self.input.recv() {
                Ok(ReaderData::Data(v)) => v,
                Ok(ReaderData::EOF) => {
                    self.closed = true;
                    return Ok(0);
                }
                Err(_) => panic!("Error in CrossbeamReader"),
            };
            self.offset = 0;
        }
        let size = min(buf.len(), self.buffer.len() - self.offset);
        buf[..size].copy_from_slice(&self.buffer[self.offset..self.offset + size]);
        self.offset += size;
        Ok(size)
    }
}

// TODO: Test if possible to make it sorted, then make it sorted...
// Called delta encoding, will give a bit better space and a small speed boost.
#[derive(bincode::Encode, bincode::Decode)]
pub enum Packed
{
    Packed(Vec<u8>),
    Remainder(Vec<u32>),
}

impl Packed
{
    pub const fn is_packed(&self) -> bool
    {
        match self {
            Packed::Packed(_) => true,
            Packed::Remainder(_) => false,
        }
    }

    pub fn unpack(&self, num_bits: u8) -> Result<Vec<u32>, String>
    {
        match self {
            Packed::Packed(packed) => {
                let mut decompressed = vec![0u32; BitPacker8x::BLOCK_LEN];
                let bitpacker = BitPacker8x::new();
                if packed.len() < (BitPacker8x::BLOCK_LEN * num_bits as usize) / 8 {
                    return Err(format!(
                        "Packed data is too small to be valid: {}, expected: {}, num_bits: {}",
                        packed.len(),
                        BitPacker8x::BLOCK_LEN * num_bits as usize,
                        num_bits
                    ));
                }
                bitpacker.decompress(packed, &mut decompressed[..], num_bits);
                Ok(decompressed)
            }
            Packed::Remainder(remainder) => Ok(remainder.clone()),
        }
    }
}

impl fmt::Debug for Packed
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result
    {
        let packed_type = match &self {
            Packed::Packed(_) => "Packed",
            Packed::Remainder(_) => "Remainder",
        };
        write!(f, "{{ packed: {:?} }}", packed_type)
    }
}

#[derive(bincode::Encode, bincode::Decode)]
pub struct Bitpacked
{
    num_bits: u8,
    packed: Packed,
}

impl fmt::Debug for Bitpacked
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result
    {
        let packed_type = match &self.packed {
            Packed::Packed(_) => "Packed",
            Packed::Remainder(_) => "Remainder",
        };
        write!(
            f,
            "Bitpacked {{ num_bits: {}, packed: {:?} }}",
            self.num_bits, packed_type
        )
    }
}

impl Bitpacked
{
    pub fn decompress(&self) -> Vec<u32>
    {
        match self.packed {
            Packed::Packed(ref packed) => {
                let mut decompressed = vec![0u32; BitPacker8x::BLOCK_LEN];
                let bitpacker = BitPacker8x::new();
                bitpacker.decompress(packed.as_ref(), &mut decompressed[..], self.num_bits);
                decompressed
            }
            Packed::Remainder(ref remainder) => remainder.clone(),
        }
    }
}

pub fn unbitpack_u32(packed: Vec<Bitpacked>) -> Vec<u32>
{
    let mut unpacked: Vec<u32> = Vec::with_capacity(packed.len() * BitPacker8x::BLOCK_LEN);

    let bitpacker = BitPacker8x::new();

    for i in packed {
        let mut decompressed = vec![0u32; BitPacker8x::BLOCK_LEN];

        match i.packed {
            Packed::Packed(x) => {
                bitpacker.decompress(&x, &mut decompressed[..], i.num_bits);
                unpacked.extend(decompressed);
            }
            Packed::Remainder(y) => {
                unpacked.extend(y);
            }
        };
    }
    unpacked
}

pub fn bitpack_u32(to_pack: &[u32]) -> (u8, Vec<Packed>)
{
    let bitpacker = BitPacker8x::new();

    let chunks = to_pack.chunks_exact(BitPacker8x::BLOCK_LEN);

    let mut bitpacked = Vec::new();

    let remainder = chunks.remainder();

    let mut num_bits: u8 = 0;

    for i in chunks {
        num_bits = std::cmp::max(num_bits, bitpacker.num_bits(i));
    }

    let chunks = to_pack.chunks_exact(BitPacker8x::BLOCK_LEN);

    for i in chunks {
        let mut packed = vec![0u8; 4 * BitPacker8x::BLOCK_LEN];
        bitpacker.compress(i, &mut packed[..], num_bits);
        bitpacked.push(Packed::Packed(packed)); // Names are hard...
                                                // Bitpacked {
                                                // num_bits,
                                                // packed: Packed::Packed(packed)
                                                //}
                                                // });
    }

    if !remainder.is_empty() {
        // bitpacked.push(Bitpacked {
        // num_bits: 0,
        // packed: Packed::Remainder(remainder.to_vec()),
        // });
        bitpacked.push(Packed::Remainder(remainder.to_vec()));
    }

    (num_bits, bitpacked)
}

// pub fn compress(ct: CompressionType, data: &[u8]) -> Vec<u8> {
//
// }

pub fn get_index_filename(filename: &str) -> String
{
    let filenamepath = Path::new(&filename);
    let filename = Path::new(filenamepath.file_name().unwrap())
        .file_stem()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned()
        + ".sfai";

    let mut path = filenamepath.parent().unwrap().to_str().unwrap().to_owned();
    if !path.is_empty() {
        path += "/";
    }

    path + &filename
}

/// Checks that the file extension ends in .sfasta or adds it if necessary
pub fn check_extension(filename: &str) -> String
{
    if !filename.ends_with(".sfasta") {
        format!("{}.sfasta", filename)
    } else {
        filename.to_string()
    }
}

// pub fn get_good_sequence_coords(seq: &[u8]) -> Vec<(usize, usize)> {
// let mut start: Option<usize> = None;
// let mut end: usize;
// let mut cur: usize = 0;
// let mut start_coords;
// let mut end_coords;
// let mut coords: Vec<(usize, usize)> = Vec::with_capacity(64);
// let results = seq.windows(3).enumerate().filter(|(_y, x)| x != &[78, 78,
// 78]).map(|(y, _x)| y);
//
// Do we need to filter the sequence at all?
// if bytecount::count(&seq, b'N') < 3 {
// coords.push((0, seq.len()));
// return coords;
// }
//
// let results = seq
// .windows(3)
// .enumerate()
// .filter(|(_y, x)| bytecount::count(&x, b'N') < 3)
// .map(|(y, _x)| y);
//
// for pos in results {
// match start {
// None => {
// start = Some(pos);
// cur = pos;
// }
// Some(_x) => (),
// };
//
// if pos - cur > 1 {
// end = cur;
// start_coords = start.unwrap();
// end_coords = end;
// coords.push((start_coords, end_coords));
// start = None;
// } else {
// cur = pos;
// }
// }
//
// Push final set of coords to the system
// if start != None {
// end = seq.len(); //cur;
// start_coords = start.unwrap();
// end_coords = end;
// if end_coords - start_coords > 1 {
// coords.push((start_coords, end_coords));
// }
// }
//
// coords
// }

// Copied from opinionated lib...
// Mutability here because we change everything to uppercase
#[inline]
pub fn capitalize_nucleotides(slice: &mut [u8])
{
    // Convert to uppercase (using, what is hopefully a fast op)
    for nucl in slice.iter_mut() {
        // *x = _convert_nucl(*x);
        match &nucl {
            65 => continue,    // A -> A
            97 => *nucl = 65,  // a -> A
            67 => continue,    // C -> C
            99 => *nucl = 67,  // c -> C
            116 => *nucl = 84, // t -> T
            84 => continue,    // T -> T
            103 => *nucl = 71, // g -> G
            71 => continue,    // G -> G
            78 => continue,    // N -> N
            // 110 => *nucl = 78, // n -> N
            _ => *nucl = 78, // Everything else -> N
        }
    }
}

#[inline]
const fn _complement_nucl(nucl: u8) -> u8
{
    // Should all be capitalized by now...
    // N -> 78
    // A -> 65
    // C -> 67
    // G -> 71
    // T -> 84
    match &nucl {
        65 => 84, // A -> T
        67 => 71, // C -> G
        84 => 65, // T -> A
        71 => 67, // G -> C
        78 => 78, // Complement of N is N
        _ => 78,  // Everything else -> N
    }
}

// Mutability here because we change everything to uppercase
/// Complement nucleotides -- Reverse is easy enough with Rust internals
pub fn complement_nucleotides(slice: &mut [u8])
{
    for x in slice.iter_mut() {
        *x = _complement_nucl(*x);
    }
}

#[inline]
pub fn get_masking(seq: &[u8]) -> Vec<bool>
{
    seq.iter().map(|&x| x > 96).collect()
}

#[cfg(test)]
mod tests
{
    use super::*;
    use std::io::BufRead;
    // #[test]
    // pub fn test_get_good_sequence_coords() {
    // let coords = get_good_sequence_coords(b"AAAAAAAAAAAAAAAAAAAANNNAAAAAAAAAAAAAAAAAAAAAAAA");
    // println!("{:#?}", coords);
    // assert!(coords == [(0, 19), (22, 47)]);
    //
    // TODO: Error, but such a minor edge case...
    // let coords =
    // get_good_sequence_coords(b"AAAAAAAAAAAAAAAAAAAANNNAAAAAAAAAAAAAAAAAAAAAAAANNN");
    // println!("{:#?}", coords);
    // assert!(coords == [(0, 19), (22, 50)]);
    //
    // TODO: Error, but such a minor edge case...
    // let coords =
    // get_good_sequence_coords(b"NNNAAAAAAAAAAAAAAAAAAAANNNAAAAAAAAAAAAAAAAAAAAAAAANNN");
    // println!("{:#?}", coords);
    // assert!(coords == [(1, 22), (25, 53)]);
    //
    // let coords = get_good_sequence_coords(b"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
    // println!("{:#?}", coords);
    // assert!(coords == [(0, 44)]);
    //
    // let coords = get_good_sequence_coords(b"AAAAAAAANAAAAAAAAANAAAAAAAAAAAAAAAAAAAAAANAA");
    // println!("{:#?}", coords);
    // assert!(coords == [(0, 44)]);
    // }
    #[test]
    pub fn test_complement_nucleotides()
    {
        let mut seq = b"AGTCCCNTNNNNTAAGATTTAGAGACCAAAAA".to_vec();
        complement_nucleotides(&mut seq);
        assert!(seq == b"TCAGGGNANNNNATTCTAAATCTCTGGTTTTT");
        seq.reverse();
        assert!(seq == b"TTTTTGGTCTCTAAATCTTANNNNANGGGACT");
    }

    #[test]
    pub fn test_capitalize_nucleotides()
    {
        let mut seq = b"agtcn".to_vec();
        capitalize_nucleotides(&mut seq);
        assert!(seq == b"AGTCN");
    }

    // Test crossbeam reader channel
    #[test]
    pub fn test_crossbeam_reader()
    {
        let (s, r) = crossbeam::channel::unbounded();
        for _ in 0..100 {
            s.send(ReaderData::Data(
                b"ACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATA".to_vec(),
            ))
            .unwrap();
        }
        s.send(ReaderData::EOF).unwrap();
        let reader = CrossbeamReader::from_channel(r);
        let bufreader = std::io::BufReader::with_capacity(8, reader);
        let all = bufreader.lines().collect::<Result<Vec<_>, _>>().unwrap();
        assert!(&all[0] == "ACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATAACTGHAGCGATCGGTGCAGCAGTGAGCTGATGCGATCGAGTCGATCGCGATA");
    }
}
