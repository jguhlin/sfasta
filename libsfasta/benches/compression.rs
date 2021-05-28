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
extern crate brotli;
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
        _ => 0x0,
    }
}

fn convert_nucleotides(n: &[u8]) -> Vec<u16> {
    n.iter().map(|x| convert_nucleotide(&x)).collect::<Vec<_>>()
}

use ux_serde::u4;

fn convert_nucleotide_u4(n: &u8) -> u4 {
    u4::new(match *n as char {
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
        _ => 0x0,
    })
}

fn convert_nucleotides_u4(n: &[u8]) -> Vec<u4> {
    n.iter()
        .map(|x| convert_nucleotide_u4(&x))
        .collect::<Vec<_>>()
}

fn serialize_as_smallints_bincode(seq: &str) -> usize {
    let converted = convert_nucleotides(seq.as_bytes());
    let converted: Vec<u32> = unsafe { std::mem::transmute(converted) };

    let bitpacker = BitPacker8x::new();

    let mut compressed_all = Vec::new();

    for chunk in converted.chunks_exact(BitPacker8x::BLOCK_LEN) {
        let num_bits: u8 = bitpacker.num_bits(&chunk);
        let mut compressed = vec![0u8; 4 * BitPacker8x::BLOCK_LEN];
        let compressed_len = bitpacker.compress(&chunk, &mut compressed[..], num_bits);
        assert_eq!(
            (num_bits as usize) * BitPacker8x::BLOCK_LEN / 8,
            compressed_len
        );
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

fn serialize_u4(seq: &str) -> usize {
    let converted = convert_nucleotides_u4(seq.as_bytes());
    let converted: Vec<u8> = unsafe { std::mem::transmute(converted) };

    let mut buf_compressed = Vec::new();
    let mut encoder = zstd::stream::write::Encoder::new(&mut buf_compressed, 6).unwrap();
    encoder.long_distance_matching(true);
    encoder.include_magicbytes(false);
    encoder.write_all(&converted).unwrap();
    let j = encoder.finish().unwrap();

    j.len()
}

fn serialize_u4_bincoded(seq: &str) -> usize {
    let converted = convert_nucleotides_u4(seq.as_bytes());
    let converted: Vec<u32> = unsafe { std::mem::transmute(converted) };

    let bitpacker = BitPacker8x::new();

    let mut compressed_all = Vec::new();

    for chunk in converted.chunks_exact(BitPacker8x::BLOCK_LEN) {
        let num_bits: u8 = bitpacker.num_bits(&chunk);
        let mut compressed = vec![0u8; 4 * BitPacker8x::BLOCK_LEN];
        let compressed_len = bitpacker.compress(&chunk, &mut compressed[..], num_bits);
        assert_eq!(
            (num_bits as usize) * BitPacker8x::BLOCK_LEN / 8,
            compressed_len
        );
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

    let size = serialize_as_smallints_bincode(seq);

    // ZSTD
    let mut buf_compressed = Vec::new();
    let mut encoder = zstd::stream::write::Encoder::new(&mut buf_compressed, 6).unwrap();
    encoder.long_distance_matching(true);
    encoder.include_magicbytes(false);
    encoder.write_all(&seq.as_bytes()).unwrap();
    let zstd = encoder.finish().unwrap();

    // BROTLI
    let mut buf_compressed = Vec::new();
    let mut compressor =
        brotli::CompressorWriter::new(&mut buf_compressed, 2 * 1024 * 1024, 11, 22);
    compressor.write_all(&seq.as_bytes()).unwrap();
    compressor.flush().unwrap();
    let buf_compressed = compressor.into_inner();
    let brotli = buf_compressed.len();
    //let brotli = compressor.finish().unwrap();

    let u4serializezstd = serialize_u4(seq);
    let u4bincodedzstd = serialize_u4_bincoded(seq);

    println!(
        "Original: {} Compressed: {} Compressed Bitpacked: {} Zstd U4: {} U4 Bincoded Zstd: {} Brotli: {}",
        seq.len(),
        zstd.len(),
        size,
        u4serializezstd,
        u4bincodedzstd,
        brotli,
    );

    // c.bench_function("unserialize_to_vec_zstd -1", |b| b.iter_with_large_drop(|| unserialize_to_vec_zstd(black_box(&buf_compressed))));
}

criterion_group! {
    name = compression_benches;
    config = Criterion::default().measurement_time(Duration::from_secs(360));
    targets = compression_benchmark
}

// criterion_group!(benches, criterion_benchmark);
criterion_main!(compression_benches);
