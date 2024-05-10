use libcompression::*;
use libfractaltree::{FractalTreeBuild, FractalTreeDisk};

use std::io::{BufRead, BufReader, BufWriter, Read, Seek};

use xxhash_rust::xxh3::xxh3_64;

trait ReadAndSeek: std::io::Read + Seek {}
impl<T: Read + Seek> ReadAndSeek for T {}

// create an index file (add .frai to extension)
pub fn create_index(input_file: &str)
{
    let output_file = format!("{}.frai", file);
    let mut file_reader = BufReader::new(std::fs::File::open(file).unwrap());
    let mut writer =
        BufWriter::new(std::fs::File::create(output_file).unwrap());

    println!("File: {}", input_file);

    let mut file_reader: Box<dyn ReadAndSeek> =
        Box::new(std::fs::File::open(input_file).unwrap());

    // Detect compression type
    let mut bytes: [u8; 10] = [0; 10];
    file_reader.read_exact(&mut bytes).unwrap();
    let compression_type = detect_compression(&bytes);

    log::debug!("Bytes: {:?}", bytes);

    let mut index = FractalTreeBuild::new(2048, 8192);

    file_reader.rewind().unwrap();

    // Extract the file (possibly block compressed)
    // record each fasta header and its offset
    let mut offset = 0;
    let mut block_offset;
    let mut header = String::new();
    let mut line = String::new();

    loop {
        block_offset = 0;
        // block_offset = file_reader.stream_position().unwrap();
        // println!("Block offset: {} Offset: {}", block_offset, offset);

        log::info!("Compression type: {:?}", compression_type);

        'inner: loop {
            let mut reader: Box<dyn BufRead> = match compression_type {
                Some(CompressionType::GZIP) => Box::new(BufReader::new(
                    flate2::read::GzDecoder::new(&mut file_reader),
                )),
                None => Box::new(BufReader::new(&mut file_reader)),
                _ => unimplemented!(),
            };

            let bytes_read = reader.read_line(&mut line).unwrap();
            offset += bytes_read as u64;
            if bytes_read == 0 {
                break 'inner;
            }

            if line.starts_with('>') {
                if !header.is_empty() {
                    // Split on first space
                    let id = header.split_whitespace().next().unwrap();
                    index
                        .insert(xxh3_64(id.as_bytes()), (block_offset, offset));
                    println!("{}: {}, {}", id, block_offset, offset);
                }
                header = line.clone();
            }
        }

        // If we didn't move at all...
        if block_offset == file_reader.stream_position().unwrap() {
            break;
        }
    }
}

fn detect_compression(bytes: &[u8; 10]) -> Option<CompressionType>
{
    // Detect DEFLATE vs ZLIB vs GZIP and panic if DEFLATE or ZLIB
    if bytes[0] == 0x78 && bytes[1] == 0x9c {
        panic!("Deflate!");
    } else if bytes[0] == 0x78 && bytes[1] == 0x01 {
        panic!("Zlib!");
    } else if bytes[0] == 0x1f && bytes[1] == 0x8b {
        Some(CompressionType::GZIP)
    } else if bytes[0] == 0x42 && bytes[1] == 0x5a {
        Some(CompressionType::BZIP2)
    } else if bytes[0] == 0x28
        && bytes[1] == 0xb5
        && bytes[2] == 0x2f
        && bytes[3] == 0xfd
    {
        Some(CompressionType::ZSTD)
    // XZ
    } else if bytes[0] == 0xfd
        && bytes[1] == 0x37
        && bytes[2] == 0x7a
        && bytes[3] == 0x58
        && bytes[4] == 0x5a
        && bytes[5] == 0x00
    {
        Some(CompressionType::XZ)
    // LZ4
    } else if bytes[0] == 0x04
        && bytes[1] == 0x22
        && bytes[2] == 0x4d
        && bytes[3] == 0x18
    {
        Some(CompressionType::LZ4)
    // BROTLI
    } else if bytes[0] == 0xce
        && bytes[1] == 0x00
        && bytes[2] == 0x00
        && bytes[3] == 0x00
    {
        Some(CompressionType::BROTLI)
    // SNAPPY
    } else if bytes[0] == 0xff
        && bytes[1] == 0x06
        && bytes[2] == 0x00
        && bytes[3] == 0x00
    {
        Some(CompressionType::SNAPPY)
    } else {
        None
    }
}

#[cfg(test)]
mod tests
{
    use super::*;

    #[test]
    pub fn test_detect_compression()
    {
        let bytes: [u8; 10] = [31, 139, 8, 8, 253, 74, 31, 102, 0, 3];
        assert_eq!(detect_compression(&bytes), Some(CompressionType::GZIP));

        let bytes: [u8; 10] = [66, 90, 104, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(detect_compression(&bytes), Some(CompressionType::BZIP2));

        let bytes: [u8; 10] = [253, 55, 122, 88, 90, 0, 0, 0, 0, 0];
        assert_eq!(detect_compression(&bytes), Some(CompressionType::XZ));

        let bytes: [u8; 10] = [4, 34, 77, 24, 0, 0, 0, 0, 0, 0];
        assert_eq!(detect_compression(&bytes), Some(CompressionType::LZ4));

        let bytes: [u8; 10] = [206, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(detect_compression(&bytes), Some(CompressionType::BROTLI));

        let bytes: [u8; 10] = [255, 6, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(detect_compression(&bytes), Some(CompressionType::SNAPPY));

        // todo add more
    }

    #[test]
    pub fn test_decompression()
    {
        let file_test = "/mnt/data/data/sfasta_testing/reads.fasta.gz";

        // Try it with box
        let mut file_reader: Box<dyn ReadAndSeek> =
            Box::new(std::fs::File::open(file_test).unwrap());

        // Read the first ten bytes then rewind
        let mut bytes: [u8; 10] = [0; 10];
        file_reader.read_exact(&mut bytes).unwrap();
        let compression_type = detect_compression(&bytes);
        println!("Compression type: {:?}", compression_type);
        println!("Bytes: {:?}", &bytes);

        file_reader.rewind().unwrap();

        let mut reader: Box<dyn BufRead> = Box::new(BufReader::new(
            flate2::read::GzDecoder::new(&mut file_reader),
        ));

        let mut line = String::new();
        let bytes_read = reader.read_line(&mut line).unwrap();
        println!("Line: {}", line);
        let bytes_read = reader.read_line(&mut line).unwrap();
        println!("Line: {}", line);
        let bytes_read = reader.read_line(&mut line).unwrap();
        println!("Line: {}", line);
        let bytes_read = reader.read_line(&mut line).unwrap();
        println!("Line: {}", line);
        let bytes_read = reader.read_line(&mut line).unwrap();
        println!("Line: {}", line);

        panic!();
    }
}
