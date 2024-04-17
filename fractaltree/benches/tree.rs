use std::ops::{AddAssign, SubAssign};

use criterion::{Throughput, black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

use bitpacking::{BitPacker, BitPacker4x, BitPacker8x};
use pulp::Arch;
use rand::prelude::*;
use xxhash_rust::xxh3::xxh3_64;
use pco::standalone::{simple_decompress, simpler_compress};

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

pub fn bitpacking8x_delta_encode(values: &mut [u32])
{
    let bitpacker = BitPacker8x::new();

    let num_bits = bitpacker.num_bits_sorted(values[0], values);

    let mut compressed: Vec<u8> = vec![0; values.len() * num_bits as usize];
    bitpacker.compress_sorted(values[0], values, &mut compressed, num_bits);
}

pub fn bitpacking4x_delta_encode(values: &mut [u32])
{
    let bitpacker = BitPacker4x::new();

    let num_bits = bitpacker.num_bits_sorted(values[0], values);

    let mut compressed: Vec<u8> = vec![0; values.len() * num_bits as usize];
    bitpacker.compress_sorted(values[0], values, &mut compressed, num_bits);
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
    pulp_delta_encode(&mut values1024);

    let mut group: criterion::BenchmarkGroup<'_, criterion::measurement::WallTime> = c.benchmark_group("Delta Decode");
    group.throughput(Throughput::Bytes(values1024.len() as u64 * std::mem::size_of::<u32>() as u64));

    group.bench_with_input(BenchmarkId::new("Pulp Arch Decode", 1024), &values1024, |b, values| {
        b.iter(|| {
            let mut values = values.clone();
            pulp_delta_decode(&mut values);
            values
        })
    });

    group.bench_with_input(BenchmarkId::new("Vanilla Decode", 1024), &values1024, |b, values| {
        b.iter(|| {
            let mut values = values.clone();
            vanilla_delta_decode(&mut values);
            values
        })
    });

    let compressed = simpler_compress(&values1024, 8).unwrap();

    group.bench_with_input(
        BenchmarkId::new("PCO Simple Decompress", 1024),
        &compressed,
        |b, values| {
            b.iter(|| {
                let mut values = values.clone();
                let v: Vec<u32> = match simple_decompress(&mut values) {
                    Ok(v) => v,
                    Err(_) => panic!("Failed to decompress"),
                };
                v
            })
        },
    );
}

pub fn bench_delta_encode(c: &mut Criterion)
{
    let values1024: Vec<u32> = (0..1024_u32).collect();

    let mut group: criterion::BenchmarkGroup<'_, criterion::measurement::WallTime> = c.benchmark_group("Delta Encode");
    group.throughput(Throughput::Bytes(values1024.len() as u64 * std::mem::size_of::<u32>() as u64));

    group.bench_with_input(BenchmarkId::new("PCO Simpler Compress", 1024), &values1024, |b, values| {
        b.iter(|| {
            let mut values = values.clone();
            match simpler_compress(&mut values, 8) {
                Ok(_) => values,
                Err(_) => panic!("Failed to compress"),
            }
        })
    });

    group.bench_with_input(BenchmarkId::new("Pulp Arch Encode", 1024), &values1024, |b, values| {
        b.iter(|| {
            let mut values = values.clone();
            pulp_delta_encode(&mut values);
            values
        })
    });

    group.bench_with_input(BenchmarkId::new("Vanilla Encode", 1024), &values1024, |b, values| {
        b.iter(|| {
            let mut values = values.clone();
            vanilla_delta_encode(&mut values);
            values
        })
    });

    group.bench_with_input(
        BenchmarkId::new("Bitpacking Encode 8x", 1024),
        &values1024,
        |b, values| {
            b.iter(|| {
                let mut values = values.clone();
                for chunk in values.chunks_mut(bitpacking::BitPacker8x::BLOCK_LEN) {
                    bitpacking8x_delta_encode(chunk);
                }
                values
            })
        },
    );

    group.bench_with_input(
        BenchmarkId::new("Bitpacking Encode 4x", 1024),
        &values1024,
        |b, values| {
            b.iter(|| {
                let mut values = values.clone();
                for chunk in values.chunks_mut(bitpacking::BitPacker4x::BLOCK_LEN) {
                    bitpacking4x_delta_encode(chunk);
                }
                values
            })
        },
    );
}

pub fn bench_large_tree(c: &mut Criterion)
{
    let mut rng = thread_rng();

    let mut values1024 = (0..1024_u64).map(|x| xxh3_64(&x.to_le_bytes())).collect::<Vec<u64>>();
    values1024.shuffle(&mut rng);
    let values1024 = black_box(values1024);

    let mut values1m = (0..(1024 * 1024_u64))
        .map(|x| xxh3_64(&x.to_le_bytes()))
        .collect::<Vec<u64>>();
    values1m.shuffle(&mut rng);
    let values1m = black_box(values1m);

    let mut values128m = (0..128_369_206_u64)
        .map(|x| xxh3_64(&x.to_le_bytes()))
        .collect::<Vec<u64>>();
    values128m.shuffle(&mut rng);
    let values128m = black_box(values128m);

    let mut group = c.benchmark_group("128m Build");
    group.sample_size(10);

    for order in [32, 64, 128, 256, 512, 1024].iter() {
        for buffer_size in [32_usize, 64_usize, 128, 256].iter() {
            group.bench_with_input(
                BenchmarkId::new("FractalTree", format!("{}-{}", order, buffer_size)),
                &(*order, *buffer_size),
                |b, (order, buffer_size)| {
                    b.iter(|| {
                        let mut tree = FractalTreeBuild::new(*order, *buffer_size);
                        for i in values128m.iter() {
                            tree.insert(*i, *i);
                        }
                        tree.flush_all();
                        tree
                    })
                },
            );
        }
    }
    group.finish();

    let mut group = c.benchmark_group("Tree Build Comparison");
    for order in [8_usize, 16, 32, 64].iter() {
        for buffer_size in [8_usize, 32, 128].iter() {
            group.bench_with_input(
                BenchmarkId::new("FractalTree", format!("{}-{}", order, buffer_size)),
                &(*order, *buffer_size),
                |b, (order, buffer_size)| {
                    b.iter(|| {
                        let mut tree = FractalTreeBuild::new(*order, *buffer_size);
                        for i in values1m.iter() {
                            tree.insert(*i, *i);
                        }
                        tree.flush_all();
                        tree
                    })
                },
            );
        }

        /*group.bench_with_input(BenchmarkId::new("SortedVec", order), order, |b, i| {
            b.iter(|| {
                let mut tree = SortedVecTree::new(*i);
                for i in values1m.iter() {
                    tree.insert(*i, *i);
                }
                tree
            })
        });

        group.bench_with_input(BenchmarkId::new("Naive Vec", order), order, |b, i| {
            b.iter(|| {
                let mut tree = BPlusTree::new(*i);
                for i in values1m.iter() {
                    tree.insert(*i, *i);
                }
                tree
            })
        }); */
    }
    group.finish();
}

pub fn bench_search(c: &mut Criterion)
{
    let mut rng = thread_rng();

    let mut values1024 = (0..1024_u64).map(|x| xxh3_64(&x.to_le_bytes())).collect::<Vec<u64>>();
    values1024.shuffle(&mut rng);
    let values1024 = black_box(values1024);

    let mut values1m = (0..(1024_u64 * 1024))
        .map(|x| xxh3_64(&x.to_le_bytes()))
        .collect::<Vec<u64>>();
    values1m.shuffle(&mut rng);
    let values1m = black_box(values1m);

    let mut values128m = (0..128_369_206_u64)
        .map(|x| xxh3_64(&x.to_le_bytes()))
        .collect::<Vec<u64>>();
    values128m.shuffle(&mut rng);
    let mut values128m = black_box(values128m);

    let mut group = c.benchmark_group("Search Tree Comparison");
    group.sample_size(500);
    group.measurement_time(std::time::Duration::from_secs(120));
    for order in [32, 64, 96, 128, 196, 256, 428, 512, 768, 1024, 2048, 4096, 8192].iter() {
        let mut tree = FractalTreeBuild::new(*order, 128);
        for i in values128m.iter() {
            tree.insert(*i, *i);
        }
        tree.flush_all();

        values128m.shuffle(&mut rng);

        group.bench_with_input(
            BenchmarkId::new("FractalTree", format!("{}", order)),
            order,
            |b, _order| {
                b.iter(|| {
                    for i in values128m.iter().step_by(1024) {
                        tree.search(i);
                    }
                })
            },
        );
    }
    group.finish();
}

criterion_group!(name = fractaltree;
    config = Criterion::default().measurement_time(std::time::Duration::from_secs(10));
    // targets = bench_large_tree, bench_search
    // targets = bench_search, bench_large_tree
    // targets = bench_large_tree, bench_search
    // targets = bench_delta_decode, bench_delta_encode, bench_search, bench_large_tree
    targets = bench_delta_encode, bench_delta_decode
);

criterion_main!(fractaltree);
