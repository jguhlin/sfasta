// Easy conversion functions
use std::fs::{metadata, File};
use std::io::{BufReader, BufWriter, Read, SeekFrom};

use crate::fasta::*;
use crate::format::Sfasta;

// TODO: Add support for metadata here...
// TODO: Will likely need to be the same builder style
pub fn convert_to_sfasta(filename: &str, output_filename: &str, block_size: u32) {
    let input = generic_open_file(filename);
    let input = Box::new(BufReader::with_capacity(512 * 1024, input.2));

    let fasta = Fasta::from_buffer(input);
    let sfasta = Sfasta::default().block_size(block_size);

    // Output file
    let out_file = File::create(output_filename.clone()).expect("Unable to write to file");
    let mut out_fh = BufWriter::with_capacity(1024 * 1024, out_file);

    bincode::serialize_into(&mut out_fh, &sfasta.directory)
        .expect("Unable to write directory to file");
    bincode::serialize_into(&mut out_fh, &sfasta.parameters)
        .expect("Unable to write Parameters to file");
    bincode::serialize_into(&mut out_fh, &sfasta.metadata)
        .expect("Unable to write Metadata to file");

    // Multithread here? Or in sequence buffer? Or in output? Or both?
}

#[inline]
pub fn generic_open_file(filename: &str) -> (usize, bool, Box<dyn Read + Send>) {
    let filesize = metadata(filename)
        .unwrap_or_else(|_| panic!("{}", &format!("Unable to open file: {}", filename)))
        .len();

    let file = match File::open(filename) {
        Err(why) => panic!("Couldn't open {}: {}", filename, why.to_string()),
        Ok(file) => file,
    };

    let file = BufReader::new(file);
    let mut compressed: bool = false;

    let fasta: Box<dyn Read + Send> = if filename.ends_with("gz") {
        compressed = true;
        Box::new(flate2::read::GzDecoder::new(file))
    } else if filename.ends_with("snappy") || filename.ends_with("sz") || filename.ends_with("sfai")
    {
        compressed = true;
        Box::new(snap::read::FrameDecoder::new(file))
    } else {
        Box::new(file)
    };

    (filesize as usize, compressed, fasta)
}
