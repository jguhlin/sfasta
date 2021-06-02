use crate::fasta::Fasta;

use std::io::{BufReader, Read};

pub fn build_dict<R: 'static>(in_buf: R) -> Vec<u8>
where
    R: Read + Send,
{
    let mut in_buf = BufReader::new(in_buf);
    let fasta = Fasta::from_buffer(in_buf.by_ref());

    let mut dict_buffer = Vec::with_capacity(32 * 1024 * 1024);
    let mut lens = Vec::with_capacity(2 * 1024 * 1024);
    for seq in fasta.into_iter() {
        dict_buffer.extend_from_slice(&seq.seq[..]);
        lens.push(seq.seq.len());

        if dict_buffer.len() >= 32 * 1024 * 1024 {
            break;
        }
    }

    let result = zstd::dict::from_continuous(&dict_buffer, &lens, 2 * 1024 * 1024).expect("Unable to build dict");
    println!("Dict created: {:#?}", result.len());
    println!();
    println!();
    
    result
}
