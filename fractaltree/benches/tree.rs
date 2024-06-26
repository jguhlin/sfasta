use std::ops::{AddAssign, SubAssign};

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion,
    Throughput,
};

use pulp::Arch;
use rand::prelude::*;
use xxhash_rust::xxh3::xxh3_64;

use libfractaltree::*;

pub fn bench_large_tree(c: &mut Criterion)
{
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

    let mut group = c.benchmark_group("128m Build");
    group.sample_size(10);

    for order in [32, 64, 128, 256, 512, 1024].iter() {
        for buffer_size in [32_usize, 64_usize, 128, 256].iter() {
            group.bench_with_input(
                BenchmarkId::new(
                    "FractalTree",
                    format!("{}-{}", order, buffer_size),
                ),
                &(*order, *buffer_size),
                |b, (order, buffer_size)| {
                    b.iter(|| {
                        let mut tree =
                            FractalTreeBuild::new(*order, *buffer_size);
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
    let mut values128m = black_box(values128m);

    let mut group = c.benchmark_group("Search Tree Comparison");
    group.sample_size(500);
    group.measurement_time(std::time::Duration::from_secs(120));
    for order in [
        32, 64, 96, 128, 196, 256, 428, 512, 768, 1024, 2048, 4096, 8192,
    ]
    .iter()
    {
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
    targets = bench_large_tree, bench_search
    // targets = bench_delta_decode, bench_delta_encode, bench_search, bench_large_tree
    // targets = bench_delta_encode, bench_delta_decode
);

criterion_main!(fractaltree);
