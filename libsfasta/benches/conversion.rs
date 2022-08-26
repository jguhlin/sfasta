use iai::{black_box, main};

use libsfasta::prelude::*;

use std::fs::{File};
use std::io::{Read};

pub fn generic_open_file(filename: &str) -> Box<dyn Read + Send> {

    let file = match File::open(filename) {
        Err(why) => panic!("Couldn't open {}: {}", filename, why),
        Ok(file) => file,
    };

    let mut compressed: bool = false;

    let fasta: Box<dyn Read + Send> = if filename.ends_with("gz") {
        compressed = true;
        Box::new(flate2::read::MultiGzDecoder::new(file))
    } else if filename.ends_with("snappy") || filename.ends_with("sz") || filename.ends_with("sfai")
    {
        compressed = true;
        Box::new(snap::read::FrameDecoder::new(file))
    } else {
        Box::new(file)
    };

    fasta
}

fn convert_erow() {
    let buf = generic_open_file("test_data/Erow_1.0.fasta.gz");

    // TODO: Handle all of the compression options...
    // TODO: Warn if more than one compression option specified
    let mut converter = Converter::default()
        .with_threads(4)
        .with_compression_type(CompressionType::ZSTD);

    let output = match File::create("bench_output/Erow_1.0.fasta.sfasta") {
        Err(why) => panic!("couldn't create: {}", why),
        Ok(file) => file,
    };

    converter.convert_fasta(buf, output);
}


fn convert_uniprot() {
    let buf = generic_open_file("test_data/uniprot_sprot.fasta.gz");

    // TODO: Handle all of the compression options...
    // TODO: Warn if more than one compression option specified
    let mut converter = Converter::default()
        .with_threads(4)
        .with_compression_type(CompressionType::ZSTD);

    let output = match File::create("bench_output/uniprot_sprot.sfasta") {
        Err(why) => panic!("couldn't create: {}", why),
        Ok(file) => file,
    };

    converter.convert_fasta(buf, output);
}

fn test() {
	println!("{}", 5 + 5);
}

// iai::main!(test);
iai::main!(convert_uniprot, convert_erow);
