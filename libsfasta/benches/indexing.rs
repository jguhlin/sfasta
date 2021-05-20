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

// const N_BINS: u64 = 16;

// use bumpalo::Bump;
// use serde_bytes::ByteBuf;

// Have 52085306 entries in NT
// Deserializing from bytebuf goes from ~33s to ... ~33s... No change.

/// This simulates reading the entire index into memory
/// Some cases will be more random access, and some will be random access but with everything open

fn unserialize_to_vec_naive(n: &Vec<u8>) -> Vec<(u64, u64)> {
    let mut j = Cursor::new(n);
    bincode::deserialize_from(&mut j).unwrap()
}

// Doesn't have an effect
fn unserialize_to_vec_bitpacker8x(n: &Vec<u8>) -> Vec<(u64, u64)> {
    let mut j = Cursor::new(n);
    bincode::deserialize_from(&mut j).unwrap()
}

// REALLY slow
fn unserialize_to_vec_zstd(n: &Vec<u8>) -> Vec<(u64, u64)> {
    let mut n = Cursor::new(n);
    let mut decoder = zstd::stream::read::Decoder::new(&mut n).unwrap();
    //decoder.multithread(16);
    //decoder.long_distance_matching(true);
    decoder.include_magicbytes(false);
    let mut buf: Vec<u8> = Vec::with_capacity(900000000);
    decoder.read_to_end(&mut buf).unwrap();
    let mut j = Cursor::new(buf);
    bincode::deserialize_from(&mut j).unwrap()
}

/// Results so far....
/// Naive, uncompressed is 306ms
/// Zstd is ~1.1secs but get size down to 600Mb from 800Mb For compression -1 and -3
/// -3 is recommended

enum HashType {
    H32,
    H64,
}

fn criterion_benchmark(c: &mut Criterion) {
    let hashtype = if (52085306 as f64 / u32::MAX as f64) < 0.05 {
        HashType::H32
    } else {
        HashType::H64
    };

    let mut j = (0_u64..52085306_u64)
        .into_iter()
        .map(|y| {
            let x: [u8; 8] = unsafe { transmute(y.to_be()) };
            let mut h: Box<dyn Hasher> = Box::new(XxHash64::with_seed(42));

            h.write(&x);
            (h.finish(), y + 10000)
        })
        .collect::<Vec<(u64, u64)>>();
    j.sort_by(|a, b| a.0.cmp(&b.0));

    let mut buf = Vec::new();
    bincode::serialize_into(&mut buf, &j).expect("Unable to serialize");
    // let buf = ByteBuf::from(buf);
    // println!("Length: {}", buf.len());
    println!(
        "Size is {} {}",
        buf.len().file_size(options::CONVENTIONAL).unwrap(),
        buf.len()
    );

    // c.bench_function("unserialize_to_vec_naive", |b| b.iter_with_large_drop(|| unserialize_to_vec_naive(black_box(&buf))));

    let hashes = j.iter().map(|(i, o)| *i).collect::<Vec<u64>>();
    let locs = j.iter().map(|(i, o)| *o).collect::<Vec<u64>>();

    let mut buf_hashes: Vec<u8> = Vec::new();
    bincode::serialize_into(&mut buf_hashes, &hashes).expect("Unable to serialize");

    let mut buf_compressed = Vec::new();
    let mut encoder = zstd::stream::write::Encoder::new(&mut buf_compressed, -3).unwrap();
    encoder.multithread(8);
    encoder.long_distance_matching(true);
    encoder.include_magicbytes(false);
    encoder.write_all(&buf_hashes).unwrap();
    encoder.finish().unwrap();

    // c.bench_function("unserialize_to_vec_zstd -1", |b| b.iter_with_large_drop(|| unserialize_to_vec_zstd(black_box(&buf_compressed))));

    //let bin_size = (hashes.len() / N_BINS).floor();
    // So bin 0 is 0..bin_size
    // Bin 1 is bin_size*1..bin_size*(1+1)
    // Bin i is bin_size*i..bin_size*(i+1)

    println!(
        "Zstd -3 Size is {}",
        buf_compressed
            .len()
            .file_size(options::CONVENTIONAL)
            .unwrap()
    );

    println!("32-bit Key Fill: {}", hashes.len() as f64 / u32::MAX as f64,);

    println!("64-bit Key Fill: {}", hashes.len() as f64 / u64::MAX as f64,);

    /*
    let mut buf_compressed = Vec::new();
    let mut encoder = zstd::stream::write::Encoder::new(&mut buf_compressed, -1).unwrap();
    encoder.multithread(16);
    encoder.long_distance_matching(true);
    encoder.include_magicbytes(false);
    encoder.write_all(&buf).unwrap();
    let buf_compressed = encoder.finish().unwrap();

    c.bench_function("unserialize_to_vec_zstd -1", |b| b.iter_with_large_drop(|| unserialize_to_vec_zstd(black_box(&buf_compressed))));

    println!(
        "Zstd -1 Size is {}",
        buf_compressed
            .len()
            .file_size(options::CONVENTIONAL)
            .unwrap()
    ); */

    // TRIED: Bitpacking, but numbers are too large (due to being hash, locs may be able to be compressed)

    // 4.0GB
    // println!("u32::MAX Size is {}", std::u32::MAX.file_size(options::CONVENTIONAL).unwrap());
    // 16 EB I think?
    // println!("u64::MAX Size is {}", std::u64::MAX.file_size(options::CONVENTIONAL).unwrap());

    // Saving bitpacking code below
    // It saves +/- 80 Mb, so savings is minimal to worse
    // But I keep trying it, so saving the code for ease of use later...

    /*
    let hashes = j.iter().map(|(i, o)| *i).collect::<Vec<u64>>();
    let locs = j.iter().map(|(i, o)| *o).collect::<Vec<u64>>();

    let mut chunker = hashes.chunks(128);
    let mut bp = BitPacker8x::new();

    let mut hashes_packed = Vec::new();
    let mut locs_packed = Vec::new();

    let mut nums: [u32; 256] = [0; 256];
    for chunk in chunker {
        for (i, j) in chunk.iter().enumerate() {
            let converted: [u32; 2] = unsafe { std::mem::transmute(j) };
            nums[i*2]   = converted[0];
            nums[i*2+1] = converted[1];
        }

        let num_bits = bp.num_bits(&nums);

        let mut compressed = vec![0u8; num_bits as usize * BitPacker8x::BLOCK_LEN / 8];
        bp.compress(&nums, &mut compressed[..], num_bits);

        let packed = BitPacked { nb: num_bits, stored: compressed };
        hashes_packed.push(packed);
    }

    let mut chunker = locs.chunks(128);

    let mut nums: [u32; 256] = [0; 256];
    for chunk in chunker {
        for (i, j) in chunk.iter().enumerate() {
            let converted: [u32; 2] = unsafe { std::mem::transmute(j) };
            nums[i*2]   = converted[0];
            nums[i*2+1] = converted[1];
        }

        let num_bits = bp.num_bits(&nums);

        let mut compressed = vec![0u8; num_bits as usize * BitPacker8x::BLOCK_LEN / 8];
        bp.compress(&nums, &mut compressed[..], num_bits);

        let packed = BitPacked { nb: num_bits, stored: compressed };
        locs_packed.push(packed);
    }

    let mut buf = Vec::new();
    //bincode::serialize_into(&mut buf, &hashes_packed).expect("Unable to serialize");
    bincode::serialize_into(&mut buf, &hashes).expect("Unable to serialize");
    bincode::serialize_into(&mut buf, &locs_packed).expect("Unable to serialize");
    // let buf = ByteBuf::from(buf);
    // println!("Length: {}", buf.len());
    println!(
        "Size is {} {}",
        buf.len().file_size(options::CONVENTIONAL).unwrap(),
        buf.len()
    ); */
}

/*
#[derive(Serialize, Deserialize)]
struct BitPacked {
    nb: u8,
    // #[serde(with = "BigArray")]
    stored: Vec<u8>,
} */

criterion_group! {
    name = benches;
    config = Criterion::default().measurement_time(Duration::from_secs(120));
    targets = criterion_benchmark
}

// criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
