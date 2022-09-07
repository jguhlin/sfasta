use std::fs::File;
use crate::conversion::Converter;
use crate::io::generic_open_file;

#[cfg(not(target_arch = "wasm32"))]
pub fn convert_fasta_file(input: &str, output: &str) {
    let (_, _, mut input_file) = generic_open_file(input);
    let mut output_fh = Box::new(File::create(output).unwrap());
    
    let converter = Converter::default()
        .with_threads(2);

    converter.convert_fasta(&mut input_file, &mut output_fh);
}