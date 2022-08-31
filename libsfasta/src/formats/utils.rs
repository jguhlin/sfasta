use std::fs::File;
use std::io::Read;

use simdutf8::basic::from_utf8;

use crate::data_types::structs::*;

/// Return type of the file format detection function
pub enum FileFormat {
    FASTA,
    FASTQ,
    SFASTA,
    GFA,
}

/// Detect the file format of a file. Prefer to use the file extension when available instead.
pub fn detect_file_format(buffer: &[u8]) -> Result<FileFormat, &'static str> {
    let buffer = from_utf8(&buffer).expect("Unable to parse file as UTF-8");
    if buffer.starts_with(">") {
        Ok(FileFormat::FASTA)
    } else if buffer.starts_with("@") {
        Ok(FileFormat::FASTQ)
    } else if buffer.starts_with("sfasta") {
        Ok(FileFormat::SFASTA)
    // Need a better test for GFA...
    } else {
        Err("Unknown file format")
    }
}

/// Return the compression type of a file
pub fn detect_compression_format(buffer: &[u8]) -> Result<CompressionType, &'static str> {
    Ok(match buffer {
        [0x1F, 0x8B, ..] => CompressionType::GZIP,
        [0x42, 0x5A, ..] => CompressionType::BZIP2,
        [0xFD, b'7', b'z', b'X', b'Z', 0x00] => CompressionType::XZ,
        [0x28, 0xB5, 0x2F, 0xFD, ..] => CompressionType::LZMA,
        [0x5D, 0x00, ..] => CompressionType::LZMA,
        [0x1F, 0x9D, ..] => CompressionType::LZMA,
        [0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C] => CompressionType::ZSTD,
        [0x04, 0x22, 0x4D, 0x18, ..] => CompressionType::LZ4,
        [0x08, 0x22, 0x4D, 0x18, ..] => CompressionType::LZ4,
        [0x52, 0x61, 0x72, 0x21, 0x1A, 0x07] => CompressionType::RAR,
        _ => return Err("File does not appear to be compressed"),
    })
}

#[cfg(tests)]
mod tests {
    use super::*;

    fn test_detect_file_format() {
        let mut file = File::open("test_data/Erow_sample.fasta").expect("Unable to open file");
        let mut buf: [u8; 10] = file.read(&mut buf).expect("Unable to read file");
        assert_eq!(detect_file_format(&buf).unwrap(), FileFormat::FASTA);

        let mut file = File::open("test_data/test.fastq").expect("Unable to open file");
        let mut buf: [u8; 10] = file.read(&mut buf).expect("Unable to read file");
        assert_eq!(detect_file_format(&buf).unwrap(), FileFormat::FASTQ);

        let buf = b"sfasta";
        assert_eq!(detect_file_format(&buf).unwrap(), FileFormat::SFASTA);
    }
}
