#![no_main]
use libfuzzer_sys::fuzz_target;
extern crate libsfasta;

use libsfasta::prelude::*;

fuzz_target!(|data: &[u8]| {
    let x = data.to_vec();
    let x = x.leak();
    let mut buf = Box::new(std::io::Cursor::new(x));
    let _ = SfastaParser::open_from_buffer(&mut buf, false);
});
