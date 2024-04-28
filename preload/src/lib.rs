use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[cfg(unix)]
use std::os::fd::AsRawFd;

use std::io::Read;

use libc::{c_char, c_int, c_long, c_uint, c_void, c_uchar, c_ulong};

enum FileCompressionType {
    Sfasta,
    Gz,
    Bz2,
    Xz,
    Zstd,
    Brotli,
    Lz4,
    Snappy,
    Plaintext,
}

fn get_file_compression_type(mut file: &std::fs::File) -> Result<FileCompressionType, std::io::Error> {
    // Read the first few bytes of the file
    let mut buffer = [0; 8];
    let bytes_read = file.read(&mut buffer).expect("Read failed");
    if bytes_read == 0 {
        // File is empty, return an error
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "File is empty",
        ));
    }

    // Check for magic numbers
    if buffer.starts_with(&[0x1f, 0x8b]) {
        return Ok(FileCompressionType::Gz);
    } else if buffer.starts_with(&[0x42, 0x5a]) {
        return Ok(FileCompressionType::Bz2);
    } else if buffer.starts_with(&[0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00]) {
        return Ok(FileCompressionType::Xz);
    } else if buffer.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        return Ok(FileCompressionType::Zstd);
    } else if buffer.starts_with(&[0x5d, 0x00, 0x00, 0x80]) {
        return Ok(FileCompressionType::Brotli);
    } else if buffer.starts_with(&[0x04, 0x22, 0x4d, 0x18]) {
        return Ok(FileCompressionType::Lz4);
    } else if buffer.starts_with(&[0xff, 0x06, 0x00, 0x00, 0x00]) {
        return Ok(FileCompressionType::Snappy);
    } if buffer.starts_with(b"SFASTA") {
        return Ok(FileCompressionType::Sfasta);
    } else {
        return Ok(FileCompressionType::Plaintext);
    }
}

enum FileType {
    FASTA,
    FASTQ,   
}

// Read the first few bytes (after decompression, if applicable) to determine the file type
fn get_file_type(in_buf: &[u8]) -> Result<FileType, std::io::Error> {
    // Check for FASTA
    if in_buf.starts_with(b">") {
        return Ok(FileType::FASTA);
    } else if in_buf.starts_with(b"@") {
        return Ok(FileType::FASTQ);
    } else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Unknown file type",
        ));
    }
}

pub struct GzFile {
    file_type: FileType,    
    buffer: Vec<u8>,
    position: usize,
    is_open: bool,
    file_handle: std::fs::File,
}

impl GzFile {
    pub fn new(path: &str, mode: &str) -> Self {
        Self {
            buffer: Vec::new(),
            position: 0,
            is_open: true,
            mode: mode.to_string(),
        }
    }

    pub fn open(path: *const c_char, mode: *const c_char) -> *mut Self {

        // File type dependent (sfasta is RA, everything else is sequential)
        #[cfg(unix)]
        nix::fcntl::posix_fadvise(
            file.as_raw_fd(),
            0,
            0,
            nix::fcntl::PosixFadviseAdvice::POSIX_FADV_SEQUENTIAL,
            // or nix::fcntl::PosixFadviseAdvice::POSIX_FADV_RANDOM,
        )
        .expect("Fadvise Failed");

        let path_str = unsafe { std::ffi::CStr::from_ptr(path).to_str().unwrap() };
        let mode_str = unsafe { std::ffi::CStr::from_ptr(mode).to_str().unwrap() };
        // Allocate and return a new GzFile instance
        Box::into_raw(Box::new(Self::new(path_str, mode_str)))
    }

    pub fn read(&mut self, buf: &mut [u8]) -> c_int {
        // Implement your custom read functionality
        0 // Placeholder
    }

    pub fn write(&mut self, buf: &[u8]) -> c_int {
        // Implement your custom write functionality
        0 // Placeholder
    }

    pub fn close(self) -> c_int {
        // Clean up resources
        0 // Placeholder
    }
}

impl Drop for GzFile {
    fn drop(&mut self) {
        // Clean up code here, if needed
    }
}

// C API wrappers to be used with LD_PRELOAD or similar mechanisms
#[no_mangle]
pub extern "C" fn gzopen(path: *const c_char, mode: *const c_char) -> *mut GzFile {
    GzFile::open(path, mode)
}

#[no_mangle]
pub extern "C" fn gzread(file: *mut GzFile, buf: *mut libc::c_void, len: libc::c_uint) -> c_int {
    let gz_file = unsafe { &mut *file };
    let buffer = unsafe { std::slice::from_raw_parts_mut(buf as *mut u8, len as usize) };
    gz_file.read(buffer)
}

#[no_mangle]
pub extern "C" fn gzwrite(file: *mut GzFile, buf: *const libc::c_void, len: libc::c_uint) -> c_int {
    let gz_file = unsafe { &mut *file };
    let buffer = unsafe { std::slice::from_raw_parts(buf as *const u8, len as usize) };
    gz_file.write(buffer)
}

#[no_mangle]
pub extern "C" fn gzclose(file: *mut GzFile) -> c_int {
    let gz_file = unsafe { Box::from_raw(file) }; // Take ownership and drop
    gz_file.close()
}

#[no_mangle]
pub extern "C" fn gzseek(file: *mut GzFile, offset: c_long, whence: c_int) -> c_long {
    // Your interposing code here
}

#[no_mangle]
pub extern "C" fn zlibVersion() -> *const c_char {
    // Your interposing code here
    // Return a custom version string
    let version = "1.3.1-custom-flate2-1.0.29-zlibng-sfasta";
    version.as_ptr() as *const c_char
}

#[no_mangle]
pub extern "C" fn uncompress(dest: *mut c_uchar, destLen: *mut c_ulong, source: *const c_uchar, sourceLen: c_ulong) -> c_int {
    // Your interposing code here
}

#[cfg(test)]
mod tests
{
    use super::*;
}
