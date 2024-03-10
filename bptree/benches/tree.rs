use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

use bumpalo::Bump;
use rand::prelude::*;
use xxhash_rust::xxh3::xxh3_64;

use libbptree::bplustree::*;
use libbptree::sorted_vec::*;
use libbptree::fractal::*;

// Todo:
// [ ] - If Fractal is faster, need to test for order & buffer size for speed!

// Early tests had bumpalo increasing performance, but that is no longer the case...

pub fn bench_large_tree(c: &mut Criterion) {
    let mut rng = thread_rng();

    let mut values = (0..1024_u64).map(|x| xxh3_64(&x.to_le_bytes())).collect::<Vec<u64>>();
    values.shuffle(&mut rng);
    let values = black_box(values);

    let mut group = c.benchmark_group("Build Tree - 1024");
    group.sample_size(500);

    //for order in [8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192].iter() {
    for order in [8, 64, 128, 256, 1024].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(order), order, |b, &order| {
            b.iter(|| {
                let mut tree = BPlusTree::new(order);
                for i in values.iter() {
                    tree.insert(i, i);
                }
                tree
            });
        });
    }
    group.finish();

    let mut group = c.benchmark_group("Build Tree SortedVec - 1024");
    group.sample_size(500);

    for order in [8, 64, 128, 256, 1024].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(order), order, |b, &order| {
            b.iter(|| {
                let mut tree = SortedVecTree::new(order);
                for i in values.iter() {
                    tree.insert(i, i);
                }
                tree
            });
        });
    }
    group.finish();

    let mut group = c.benchmark_group("Build Tree Fractal - 1024");
    group.sample_size(500);

    for order in [8, 64, 128, 256, 1024].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(order), order, |b, &order| {
            b.iter(|| {
                let mut tree = FractalTree::new(order, order);
                for i in values.iter() {
                    tree.insert(i, i);
                }
                tree.flush_all();
                tree
            });
        });
    }
    group.finish();

    let mut group = c.benchmark_group("Build Tree - 1 Million Elements");
    group.sample_size(20);

    let mut values: Vec<u64> = (0..(1024 * 1024_u64)).map(|x| xxh3_64(&x.to_le_bytes())).collect::<Vec<u64>>();
    values.shuffle(&mut rng);
    let values = black_box(values);

    for order in [8, 64, 128, 256, 1024].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(order), order, |b, &order| {
            b.iter(|| {
                let mut tree = BPlusTree::new(order);
                for i in values.iter() {
                    tree.insert(i, i);
                }
                tree
            });
        });
    }
    group.finish();

    let mut group = c.benchmark_group("Build Tree SortedVec - 1 Million Elements");
    group.sample_size(20);

    for order in [8, 64, 128, 256, 1024].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(order), order, |b, &order| {
            b.iter(|| {
                let mut tree = SortedVecTree::new(order);
                for i in values.iter() {
                    tree.insert(i, i);
                }
                tree
            });
        });
    }
    group.finish();

    let mut group = c.benchmark_group("Build Tree Fractal - 1 Million Elements");
    group.sample_size(20);

    for order in [8, 64, 128, 256, 1024].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(order), order, |b, &order| {
            b.iter(|| {
                let mut tree = FractalTree::new(order, order);
                for i in values.iter() {
                    tree.insert(i, i);
                }
                tree.flush_all();
                tree
            });
        });
    }
    group.finish();

    let mut values: Vec<u64> = (0..128_369_206_u64).map(|x| xxh3_64(&x.to_le_bytes())).collect::<Vec<u64>>();
    values.shuffle(&mut rng);
    let values = black_box(values);

    let mut group = c.benchmark_group("Build Tree Fractal - 128_369_206 Elements");
    group.sample_size(2);

    for order in [8, 64, 128, 256, 1024].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(order), order, |b, &order| {
            b.iter(|| {
                let mut tree = FractalTree::new(order, order);
                for i in values.iter() {
                    tree.insert(i, i);
                }
                tree.flush_all();
                tree
            });
        });
    }
    group.finish();


    let mut group = c.benchmark_group("Build Tree SortedVec - 128_369_206 Elements");
    group.sample_size(2);

    for order in [8, 64, 128, 256, 1024].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(order), order, |b, &order| {
            b.iter(|| {
                let mut tree = SortedVecTree::new(order);
                for i in values.iter() {
                    tree.insert(i, i);
                }
                tree
            });
        });
    }
    group.finish();

    let mut group = c.benchmark_group("Build Tree - 128_369_206 Elements");
    group.sample_size(2);

    for order in [8, 64, 128, 256, 1024].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(order), order, |b, &order| {
            b.iter(|| {
                let mut tree = BPlusTree::new(order);
                for i in values.iter() {
                    tree.insert(i, i);
                }
                tree
            });
        });
    }
    group.finish();
}

pub fn bench_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("Search Tree - 64");
    group.sample_size(500);

    for order in [8, 64, 128, 256, 1024].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(order), order, |b, &order| {
            let mut tree = BPlusTree::new(order);
            for i in 0..64_u64 {
                tree.insert(i, i);
            }
            b.iter(|| {
                for i in 0..64_u64 {
                    tree.search(i);
                }
            })
        });
    }
    group.finish();

    let mut group = c.benchmark_group("Search Tree - 1024");
    group.sample_size(500);

    for order in [8, 64, 128, 256, 1024].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(order), order, |b, &order| {
            let mut tree = BPlusTree::new(order);
            for i in 0..1024_u64 {
                tree.insert(i, i);
            }
            b.iter(|| {
                for i in 0..64_u64 {
                    tree.search(i);
                }
            })
        });
    }
    group.finish();

    let mut group = c.benchmark_group("Search Tree - 1 Million Elements");
    group.sample_size(100);

    for order in [8, 64, 128, 256, 1024].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(order), order, |b, &order| {
            let mut tree = BPlusTree::new(order);
            for i in 0..1024 * 1024 {
                tree.insert(i as u64, i as u64);
            }
            b.iter(|| {
                for i in 0..1024 * 1024 {
                    tree.search(i as u64);
                }
            })
        });
    }
    group.finish();

    let mut group = c.benchmark_group("Search Tree - 128_369_206 Elements");
    group.sample_size(100);

    for order in [8, 64, 128, 256, 1024].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(order), order, |b, &order| {
            let mut tree = BPlusTree::new(order);
            for i in 0..128_369_206 {
                tree.insert(i as u64, i as u64);
            }
            b.iter(|| {
                for i in (0..128_369_206).step_by(1000) {
                    tree.search(i as u64);
                }
            })
        });
    }
    group.finish();
}

criterion_group!(name = add_locs_large;
    config = Criterion::default().measurement_time(std::time::Duration::from_secs(10));
    // targets = bench_large_tree, bench_search
    // targets = bench_search, bench_large_tree
    targets = bench_large_tree
);

// criterion_main!(add_locs, add_locs_large);
criterion_main!(add_locs_large);
