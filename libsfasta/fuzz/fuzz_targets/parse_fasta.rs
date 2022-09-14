#![no_main]
use libfuzzer_sys::fuzz_target;
extern crate libsfasta;

fuzz_target!(|data: &[u8]| {
    let mut buf = std::io::BufReader::new(data);
    let fasta = libsfasta::prelude::Fasta::from_buffer(&mut buf);
    while let Some(_) fasta.iter().next() {
        true
    }
});
