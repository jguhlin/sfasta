#![no_main]
use libfuzzer_sys::fuzz_target;
extern crate libsfasta;

fuzz_target!(|data: &[u8]| {
    let mut buf = data.to_vec();
    let _ = libsfasta::prelude::Fasta::from_buffer(&mut buf);
});
