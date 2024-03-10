use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

use bumpalo::Bump;

use libbptree::bplustree::*;
use libbptree::fractal::*;

pub fn bench_large_tree(c: &mut Criterion) {
    let mut group = c.benchmark_group("Build Tree - 64");
    group.sample_size(500);

    for order in [8, 64, 256, 1024, 8192, 16384, 32768, 65536, 131072].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(order), order, |b, &order| {
            b.iter(|| {
                let mut tree = BPlusTree::new(order);
                for i in 0..64_u64 {
                    tree.insert(i, i);
                }
                tree
            });
        });
    }
    group.finish();

    /*

    Not worth it until working...

    let mut group = c.benchmark_group("Build Tree Fractal - 64");
    group.sample_size(500);

    for order in [8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(order), order, |b, &order| {
            b.iter(|| {
                let mut tree = FractalTree::new(order, order / 2);
                for i in 0..64_u64 {
                    tree.insert(i, i);
                }
                tree
            });
        });
    }
    group.finish(); */

    let mut group = c.benchmark_group("Build Tree - 1024");
    group.sample_size(500);

    //for order in [8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192].iter() {
    for order in [8, 64, 256, 1024, 8192, 16384, 32768, 65536, 131072].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(order), order, |b, &order| {
            b.iter(|| {
                let mut tree = BPlusTree::new(order);
                for i in 0..1024_u64 {
                    tree.insert(i, i);
                }
                tree
            });
        });
    }
    group.finish();

    let mut group = c.benchmark_group("Build Tree - 1 Million Elements");
    group.sample_size(20);

    //for order in [8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192].iter() {
    for order in [8, 64, 256, 1024, 8192, 16384, 32768, 65536, 131072].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(order), order, |b, &order| {
            b.iter(|| {
                let mut tree = BPlusTree::new(order);
                for i in 0..1024 * 1024_u64 {
                    tree.insert(i, i);
                }
                tree
            });
        });
    }
    group.finish();

    let mut group = c.benchmark_group("Build Tree Bumpalo - 1 Million Elements");
    group.sample_size(20);

    //for order in [8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192].iter() {
    for order in [8, 64, 256, 1024, 8192, 16384, 32768, 65536, 131072].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(order), order, |b, &order| {
            b.iter(|| {
                let bump = Bump::new();
                let tree = bump.alloc(BPlusTree::new(order));
                for i in 0..1024 * 1024_u64 {
                    tree.insert(i, i);
                }
            });
        });
    }
    group.finish();

    let mut group = c.benchmark_group("Build Tree - 128_369_206 Elements");
    group.sample_size(20);

    // for order in [8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192].iter() {
    for order in [8, 64, 256, 1024, 8192, 16384, 32768, 65536, 131072].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(order), order, |b, &order| {
            b.iter(|| {
                let mut tree = BPlusTree::new(order);
                for i in 0..128_369_206_u64 {
                    tree.insert(i, i);
                }
                tree
            });
        });
    }
    group.finish();

    let mut group = c.benchmark_group("Build Tree Bumpalo - 128_369_206 Elements");
    group.sample_size(20);

    // for order in [8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192].iter() {
    for order in [8, 64, 256, 1024, 8192, 16384, 32768, 65536, 131072].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(order), order, |b, &order| {
            b.iter(|| {
                let bump = Bump::new();
                let tree = bump.alloc(BPlusTree::new(order));
                for i in 0..128_369_206_u64 {
                    tree.insert(i, i);
                }
                bump
            });
        });
    }
    group.finish();
}

pub fn bench_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("Search Tree - 64");
    group.sample_size(500);

    // for order in [32, 64, 128, 256, 512, 1024, 2048, 4096, 8192].iter() {
    for order in [8, 64, 256, 1024, 8192, 16384, 32768, 65536, 131072].iter() {
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

    // for order in [32, 64, 128, 256, 512, 1024, 2048, 4096, 8192].iter() {
    for order in [8, 64, 256, 1024, 8192, 16384, 32768, 65536, 131072].iter() {
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


    //for order in [32, 64, 128, 256, 512, 1024, 2048, 4096, 8192].iter() {
    for order in [8, 64, 256, 1024, 8192, 16384, 32768, 65536, 131072].iter() {
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

    //for order in [8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192].iter() {
    for order in [8, 64, 256, 1024, 8192, 16384, 32768, 65536, 131072].iter() {
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

    let mut group = c.benchmark_group("Search Tree Bumpalo - 128_369_206 Elements");
    group.sample_size(100);

    // for order in [8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192].iter() {
    for order in [8, 64, 256, 1024, 8192, 16384, 32768, 65536, 131072].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(order), order, |b, &order| {
            let bump = Bump::new();
            let tree = bump.alloc(BPlusTree::new(order));
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
