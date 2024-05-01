use crate::libcompression::*;
use fractaltree::{FractalTreeBuild, FractalTreeDisk};

use std::io::{BufRead, BufReader, BufWriter, Read, Write};

use flate2::prelude::*;
use xxhash_rust::xxh3::xxh3_64;

// create an index file (add .frai to extension)
pub fn create_index(file: &str)
{
    let output_file = format!("{}.fai", file);
    let mut file_reader = BufReader::new(std::fs::File::open(file).unwrap());
    let mut writer =
        BufWriter::new(std::fs::File::create(output_file).unwrap());

    let mut index = FractalTreeBuild::new(2048, 8192);

    // Grab the first eight bytes without consuming them
    let data = reader.fill_buf().unwrap();

    let compression_type = detect_compression(&data[0..8]);

    // Extract the file (possibly block compressed)
    // record each fasta header and its offset
    let mut offset = 0;
    let mut block_offset = 0;
    let mut header = String::new();
    let mut headers = Vec::new();
    let mut line = String::new();

    loop {
        block_offset = reader.current_pos();

        'inner: loop {
            let mut reader = match compression_type {
                CompressionType::Gzip => flate2::GzDecoder::new(file_reader),
                _ => unimplemented!(),
            };

            let bytes_read = reader.read_line(&mut line).unwrap();
            if bytes_read == 0 {
                file_reader = reader.into_inner();
                break 'inner;
            }

            if line.starts_with('>') {
                if !header.is_empty() {
                    // Split on first space
                    let id = header.split_whitespace().next().unwrap();
                    index.add(xxh3_64(id), (block_offset, offset));
                }
                header = line;
            }
        }

        if block_offset == reader.current_pos() {
            break;
        }
    }
}

fn detect_compression(bytes: &[u8; 8]) -> Option<CompressionType>
{
    if bytes[0] == 0x1f && bytes[1] == 0x8b {
        Some(CompressionType::Gzip)
    } else if bytes[0] == 0x42 && bytes[1] == 0x5a {
        Some(CompressionType::Bzip2)
    } else if bytes[0] == 0xfd
        && bytes[1] == 0x37
        && bytes[2] == 0x7a
        && bytes[3] == 0x58
        && bytes[4] == 0x5a
        && bytes[5] == 0x00
    {
        Some(CompressionType::Lzma)
    } else if bytes[0] == 0x28
        && bytes[1] == 0xb5
        && bytes[2] == 0x2f
        && bytes[3] == 0xfd
    {
        Some(CompressionType::Zstd)
    // XZ
    } else if bytes[0] == 0xfd
        && bytes[1] == 0x37
        && bytes[2] == 0x7a
        && bytes[3] == 0x58
        && bytes[4] == 0x5a
        && bytes[5] == 0x00
    {
        Some(CompressionType::Xz)
    // LZ4
    } else if bytes[0] == 0x04
        && bytes[1] == 0x22
        && bytes[2] == 0x4d
        && bytes[3] == 0x18
    {
        Some(CompressionType::Lz4)
    // BROTLI
    } else if bytes[0] == 0xce
        && bytes[1] == 0x00
        && bytes[2] == 0x00
        && bytes[3] == 0x00
    {
        Some(CompressionType::Brotli)
    // SNAPPY
    } else if bytes[0] == 0xff
        && bytes[1] == 0x06
        && bytes[2] == 0x00
        && bytes[3] == 0x00
    {
        Some(CompressionType::Snappy)
    } else {
        None
    }
}
