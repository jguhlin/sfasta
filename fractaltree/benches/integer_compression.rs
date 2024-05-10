use std::ops::{AddAssign, SubAssign};

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion,
    Throughput,
};

use bitpacking::{BitPacker, BitPacker4x, BitPacker8x};
use pco::standalone::{simple_compress, simple_decompress};
use pulp::Arch;
use rand::prelude::*;
use xxhash_rust::xxh3::xxh3_64;

use stream_vbyte::{
    decode::{cursor::DecodeCursor, decode},
    encode::encode,
    scalar::Scalar,
};

use libfractaltree::*;

pub fn pulp_delta_encode<T>(values: &mut [T])
where
    T: Default + num::traits::Unsigned + Copy + SubAssign,
{
    let arch = Arch::new();

    arch.dispatch(|| {
        let mut prev: T = Default::default();
        for i in values {
            let tmp = *i;
            *i -= prev;
            prev = tmp;
        }
    });
}

pub fn vanilla_delta_encode<T>(values: &mut [T])
where
    T: Default + num::traits::Unsigned + Copy + SubAssign,
{
    let mut prev: T = Default::default();
    for i in values {
        let tmp = *i;
        *i -= prev;
        prev = tmp;
    }
}

pub fn bitpacking8x_delta_encode(values: &[u32]) -> Vec<u8>
{
    let bitpacker = BitPacker8x::new();

    let num_bits = bitpacker.num_bits_sorted(values[0], values);

    let mut compressed: Vec<u8> = vec![0; values.len() * num_bits as usize];
    bitpacker.compress_sorted(values[0], values, &mut compressed, num_bits);
    compressed
}

pub fn bitpacking4x_delta_encode(values: &[u32]) -> Vec<u8>
{
    let bitpacker = BitPacker4x::new();

    let num_bits = bitpacker.num_bits_sorted(values[0], values);

    let mut compressed: Vec<u8> = vec![0; values.len() * num_bits as usize];
    bitpacker.compress_sorted(values[0], values, &mut compressed, num_bits);
    compressed
}

pub fn pulp_delta_decode<T>(values: &mut [T])
where
    T: Default + num::traits::Unsigned + Copy + AddAssign,
{
    let arch = Arch::new();

    arch.dispatch(|| {
        let mut prev: T = Default::default();
        for i in values {
            *i += prev;
            prev = *i;
        }
    });
}

pub fn vanilla_delta_decode<T>(values: &mut [T])
where
    T: Default + num::traits::Unsigned + Copy + AddAssign,
{
    let mut prev: T = Default::default();
    for i in values {
        *i += prev;
        prev = *i;
    }
}

pub fn bench_delta_decode(c: &mut Criterion)
{
    let mut values1024: Vec<u32> = (0..1024_u32).collect();
    let pco_config = pco::ChunkConfig::default()
        .with_compression_level(8)
        .with_delta_encoding_order(Some(1));
    let compressed = simple_compress(&values1024, &pco_config).unwrap();

    pulp_delta_encode(&mut values1024);

    let mut group: criterion::BenchmarkGroup<
        '_,
        criterion::measurement::WallTime,
    > = c.benchmark_group("Delta Decode - Inclusive Range 0..1024");
    group.throughput(Throughput::Bytes(
        values1024.len() as u64 * std::mem::size_of::<u32>() as u64,
    ));

    group.bench_with_input(
        BenchmarkId::new("Pulp Arch Decode", 1024),
        &values1024,
        |b, values| {
            b.iter(|| {
                let mut values = values.clone();
                pulp_delta_decode(&mut values);
                values
            })
        },
    );

    group.bench_with_input(
        BenchmarkId::new("Vanilla Decode", 1024),
        &values1024,
        |b, values| {
            b.iter(|| {
                let mut values = values.clone();
                vanilla_delta_decode(&mut values);
                values
            })
        },
    );

    group.bench_with_input(
        BenchmarkId::new("PCO Simple Decompress", 1024),
        &compressed,
        |b, values| {
            b.iter(|| {
                let v: Vec<u32> = match simple_decompress(&values) {
                    Ok(v) => v,
                    Err(_) => panic!("Failed to decompress"),
                };
                v
            })
        },
    );

    group.finish();

    let mut values1024: Vec<u32> = (0..1024 * 1024_u32).collect();
    let compressed = simple_compress(&values1024, &pco_config).unwrap();
    pulp_delta_encode(&mut values1024);

    let mut group: criterion::BenchmarkGroup<
        '_,
        criterion::measurement::WallTime,
    > = c.benchmark_group("Delta Decode - Inclusive Range 0..(1024*1024)");
    group.throughput(Throughput::Bytes(
        values1024.len() as u64 * std::mem::size_of::<u32>() as u64,
    ));

    group.bench_with_input(
        BenchmarkId::new("Pulp Arch Decode", 1024),
        &values1024,
        |b, values| {
            b.iter(|| {
                let mut values = values.clone();
                pulp_delta_decode(&mut values);
                values
            })
        },
    );

    group.bench_with_input(
        BenchmarkId::new("Vanilla Decode", 1024),
        &values1024,
        |b, values| {
            b.iter(|| {
                let mut values = values.clone();
                vanilla_delta_decode(&mut values);
                values
            })
        },
    );

    group.bench_with_input(
        BenchmarkId::new("PCO Simple Decompress", 1024),
        &compressed,
        |b, values| {
            b.iter(|| {
                let v: Vec<u32> = match simple_decompress(&values) {
                    Ok(v) => v,
                    Err(_) => panic!("Failed to decompress"),
                };
                v
            })
        },
    );

    group.finish();

    let mut values1024: Vec<u32> = (0..1024 * 1024_u32)
        .map(|x| xxh3_64(&x.to_le_bytes()) as u32)
        .collect();
    values1024.sort_unstable();
    let compressed = simple_compress(&values1024, &pco_config).unwrap();
    pulp_delta_encode(&mut values1024);

    let mut group: criterion::BenchmarkGroup<
        '_,
        criterion::measurement::WallTime,
    > = c.benchmark_group("Delta Decode - Xxh3 as u32 1024*1024 Elements");
    group.throughput(Throughput::Bytes(
        values1024.len() as u64 * std::mem::size_of::<u32>() as u64,
    ));

    group.bench_with_input(
        BenchmarkId::new("Pulp Arch Decode", 1024),
        &compressed,
        |b, values| {
            b.iter(|| {
                let mut values = values.clone();
                pulp_delta_decode(&mut values);
                values
            })
        },
    );

    group.bench_with_input(
        BenchmarkId::new("Vanilla Decode", 1024),
        &compressed,
        |b, values| {
            b.iter(|| {
                let mut values = values.clone();
                vanilla_delta_decode(&mut values);
                values
            })
        },
    );

    let compressed = simple_compress(&values1024, &pco_config).unwrap();

    group.bench_with_input(
        BenchmarkId::new("PCO Simple Decompress", 1024),
        &compressed,
        |b, values| {
            b.iter(|| {
                let v: Vec<u32> = match simple_decompress(&values) {
                    Ok(v) => v,
                    Err(_) => panic!("Failed to decompress"),
                };
                v
            })
        },
    );

    group.finish();
}

pub fn bench_delta_encode_u32(c: &mut Criterion)
{
    let values1024: Vec<u32> = (0..1024_u32).collect();

    let values1mil: Vec<u32> = (0..1024 * 1024_u32).collect();

    let mut values1024xxh3: Vec<u32> = (0..1024_u32)
        .map(|x| xxh3_64(&x.to_le_bytes()) as u32)
        .collect();
    values1024xxh3.sort_unstable();

    let mut values1milxxh3: Vec<u32> = (0..1024 * 1024_u32)
        .map(|x| xxh3_64(&x.to_le_bytes()) as u32)
        .collect();
    values1milxxh3.sort_unstable();

    let mut group = c.benchmark_group("Delta Encode");

    for input in [
        ("1024 Elem Inclusive", values1024),
        ("1mil Elem Inclusive", values1mil),
        ("1024 Elem Xxh3", values1024xxh3),
        ("1mil Elem Xxh3", values1milxxh3),
    ] {
        let data = input.1;

        group.throughput(Throughput::Bytes(
            data.len() as u64 * std::mem::size_of::<u32>() as u64,
        ));

        group.bench_with_input(
            BenchmarkId::new("Stream VByte Crate", input.0),
            &data,
            |b, values| {
                b.iter(|| {
                    let mut encoded_data = Vec::new();
                    // make some space to encode into
                    encoded_data.resize(5 * values.len(), 0x0);

                    // use Scalar implementation that works on any hardware
                    encode::<Scalar>(&values, &mut encoded_data);
                    encoded_data
                })
            },
        );

        let config = bincode::config::standard().with_variable_int_encoding();

        group.bench_with_input(
            BenchmarkId::new("Plain Bincode", input.0),
            &data,
            |b, values| {
                b.iter(|| bincode::encode_to_vec(&values, config).unwrap())
            },
        );

        let pco_config = pco::ChunkConfig::default()
            .with_compression_level(8)
            .with_delta_encoding_order(Some(1));

        group.bench_with_input(
            BenchmarkId::new("PCO Simpler Compress", input.0),
            &data,
            |b, values| {
                b.iter(|| match simple_compress(&values, &pco_config) {
                    Ok(_) => values,
                    Err(_) => panic!("Failed to compress"),
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("Pulp Arch Encode", input.0),
            &data,
            |b, values| {
                b.iter(|| {
                    let mut values = values.clone();
                    pulp_delta_encode(&mut values);
                    values
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("Vanilla Encode", input.0),
            &data,
            |b, values| {
                b.iter(|| {
                    let mut values = values.clone();
                    vanilla_delta_encode(&mut values);
                    values
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("Bitpacking Encode 8x", input.0),
            &data,
            |b, values| {
                b.iter(|| {
                    for chunk in
                        values.chunks(bitpacking::BitPacker8x::BLOCK_LEN)
                    {
                        bitpacking8x_delta_encode(chunk);
                    }
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("Bitpacking Encode 4x", input.0),
            &data,
            |b, values| {
                b.iter(|| {
                    for chunk in
                        values.chunks(bitpacking::BitPacker4x::BLOCK_LEN)
                    {
                        bitpacking4x_delta_encode(chunk);
                    }
                })
            },
        );
    }

    group.finish();
}

pub fn bench_delta_encode_u8(c: &mut Criterion)
{
    let fastq_illumina_scores =
        include_bytes!("data/fastq_scores_illumina.txt").to_vec();
    let fastq_illumina_scores_1024 = fastq_illumina_scores[..1024].to_vec();
    let fastq_illumina_scores_2048 = fastq_illumina_scores[..2048].to_vec();

    let mut group = c.benchmark_group("Delta Encode");

    for input in [
        ("FASTQ Quality Scores Illumina", fastq_illumina_scores),
        (
            "FASTQ Quality Scores Illumina 1024",
            fastq_illumina_scores_1024,
        ),
        (
            "FASTQ Quality Scores Illumina 2048",
            fastq_illumina_scores_2048,
        ),
    ] {
        let data = input.1;

        group.throughput(Throughput::Bytes(
            data.len() as u64 * std::mem::size_of::<u32>() as u64,
        ));

        let config = bincode::config::standard().with_variable_int_encoding();

        group.bench_with_input(
            BenchmarkId::new("Plain Bincode", input.0),
            &data,
            |b, values| {
                b.iter(|| bincode::encode_to_vec(&values, config).unwrap())
            },
        );

        let mut data = data.to_vec();

        group.bench_with_input(
            BenchmarkId::new("Pulp Arch Encode", input.0),
            &data,
            |b, values| {
                b.iter(|| {
                    let mut values = values.clone();
                    pulp_delta_encode(&mut values);
                    values
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("Vanilla Encode", input.0),
            &data,
            |b, values| {
                b.iter(|| {
                    let mut values = values.clone();
                    vanilla_delta_encode(&mut values);
                    values
                })
            },
        );
    }

    group.finish();
}

criterion_group!(name = integercompression;
    config = Criterion::default().measurement_time(std::time::Duration::from_secs(20));
    targets = bench_delta_encode_u8 // bench_delta_encode_u32 // , bench_delta_decode
);

criterion_main!(integercompression);
