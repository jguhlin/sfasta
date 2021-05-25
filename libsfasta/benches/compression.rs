
use bitpacking::{BitPacker, BitPacker8x};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use humansize::{file_size_opts as options, FileSize};
use std::hash::Hasher;
use std::io::Cursor;
use std::io::Read;
use std::io::Write;
use std::mem::transmute;
use std::time::Duration;
use twox_hash::{XxHash32, XxHash64};
use zstd::stream::write::Encoder;
extern crate num_traits;
#[macro_use]
extern crate generic_array;
#[macro_use]
extern crate numeric_array;
use generic_array::typenum::consts::U256;
use std::ops::Sub;
extern crate serde;

use serde::{Deserialize, Serialize};

use numeric_array::{NumericArray, NumericConstant};

// https://github.com/KirillKryukov/naf/blob/master/NAFv2.pdf
const fn convert_nucleotide(n: &u8) -> u16 {
    match *n as char {
        'A' => 0x8,
        'C' => 0x4,
        'G' => 0x2,
        'T' => 0x1,
        'N' => 0xF,
        'U' => 0x1,
        'R' => 0xA,
        'Y' => 0x5,
        'S' => 0x6,
        'W' => 0x9,
        'K' => 0x3,
        'M' => 0xC,
        'B' => 0x7,
        'D' => 0xB,
        'H' => 0xD,
        'V' => 0xE,
        _    => 0x0,
    }
}

fn convert_nucleotides(n: &[u8]) -> Vec<u16> {
    n.iter().map(|x| convert_nucleotide(&x)).collect::<Vec<_>>()
}

fn serialize_as_smallints(seq: &str) -> usize {

    let converted = convert_nucleotides(seq.as_bytes());
    let converted: Vec<u32> = unsafe {
        std::mem::transmute(converted)
    };

    let bitpacker = BitPacker8x::new();

    let mut compressed_all = Vec::new();

    for chunk in converted.chunks_exact(BitPacker8x::BLOCK_LEN) {
        let num_bits: u8 = bitpacker.num_bits(&chunk);
        let mut compressed = vec![0u8; 4 * BitPacker8x::BLOCK_LEN];
        let compressed_len = bitpacker.compress(&chunk, &mut compressed[..], num_bits);
        assert_eq!((num_bits as usize) *  BitPacker8x::BLOCK_LEN / 8, compressed_len);
        compressed_all.extend_from_slice(&compressed[..]);
    }

    let mut buf_compressed = Vec::new();
    let mut encoder = zstd::stream::write::Encoder::new(&mut buf_compressed, 6).unwrap();
    encoder.multithread(8);
    encoder.long_distance_matching(true);
    encoder.include_magicbytes(false);
    encoder.write_all(&compressed_all).unwrap();
    let j = encoder.finish().unwrap();

    j.len()

}

fn compression_benchmark(c: &mut Criterion) {

    let seq = include_str!("../test_data/test_sequence.fasta");

    let size = serialize_as_smallints(seq);

    let mut buf_compressed = Vec::new();
    let mut encoder = zstd::stream::write::Encoder::new(&mut buf_compressed, 6).unwrap();
    encoder.multithread(8);
    encoder.long_distance_matching(true);
    encoder.include_magicbytes(false);
    encoder.write_all(&seq.as_bytes()).unwrap();
    let j = encoder.finish().unwrap();

    println!("Original: {} Compressed: {} Compressed Bitpacked: {}", seq.len(), j.len(), size);
    
    // c.bench_function("unserialize_to_vec_zstd -1", |b| b.iter_with_large_drop(|| unserialize_to_vec_zstd(black_box(&buf_compressed))));

}

criterion_group! {
    name = compression_benches;
    config = Criterion::default().measurement_time(Duration::from_secs(240));
    targets = compression_benchmark
}

// criterion_group!(benches, criterion_benchmark);
criterion_main!(compression_benches);
