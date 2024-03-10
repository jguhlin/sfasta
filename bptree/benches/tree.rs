use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

use bumpalo::Bump;
use rand::prelude::*;
use xxhash_rust::xxh3::xxh3_64;

use libbptree::bplustree::*;
use libbptree::fractal::*;
use libbptree::sorted_vec::*;

// Todo:
// [ ] - If Fractal is faster, need to test for order & buffer size for speed!

// Early tests had bumpalo increasing performance, but that is no longer the case...

pub fn bench_large_tree(c: &mut Criterion) {
    let mut rng = thread_rng();

    let mut values1024 = (0..1024_u64)
        .map(|x| xxh3_64(&x.to_le_bytes()))
        .collect::<Vec<u64>>();
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

    let mut group = c.benchmark_group("Tree Build Comparison");
    for order in [8_usize, 16, 32, 64].iter() {
        group.bench_with_input(BenchmarkId::new("SortedVec", order), order, 
            |b, i| b.iter(|| 
                {
                    let mut tree = SortedVecTree::new(*i);
                    for i in values1m.iter() {
                        tree.insert(i, i);
                    }
                    tree
                }
            ));

            for buffer_size in [8_usize, 16, 32, 64, 128].iter() {
                group.bench_with_input(BenchmarkId::new("FractalTree", format!("{}-{}", order, buffer_size)), &(*order, *buffer_size), 
                |b, (order, buffer_size)| b.iter(|| 
                    {
                        let mut tree = FractalTree::new(*order, *buffer_size);
                        for i in values1m.iter() {
                            tree.insert(i, i);
                        }
                        tree.flush_all();
                        tree
                    }
                ));
            }

            group.bench_with_input(BenchmarkId::new("Naive Vec", order), order, 
            |b, i| b.iter(|| 
                {
                    let mut tree = BPlusTree::new(*i);
                    for i in values1m.iter() {
                        tree.insert(i, i);
                    }
                    tree
                }
            ));
    }
    group.finish();

}

pub fn bench_search(c: &mut Criterion) {
    let mut rng = thread_rng();

    let mut values1024 = (0..1024_u64)
        .map(|x| xxh3_64(&x.to_le_bytes()))
        .collect::<Vec<u64>>();
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
    let values128m = black_box(values128m);

    let mut group = c.benchmark_group("Search Tree - 1024");
    group.sample_size(500);

    for order in [8, 64, 128, 256, 1024].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(order), order, |b, &order| {
            let mut tree = BPlusTree::new(order);
            for i in values1024.iter() {
                tree.insert(*i, *i);
            }
            b.iter(|| {
                for i in values1024.iter() {
                    tree.search(*i);
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
            for i in values1m.iter() {
                tree.insert(*i, *i);
            }
            b.iter(|| {
                for i in values1m.iter() {
                    tree.search(*i);
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
            for i in values128m.iter() {
                tree.insert(*i, *i);
            }
            b.iter(|| {
                for i in values128m.iter().step_by(1000) {
                    tree.search(*i);
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
