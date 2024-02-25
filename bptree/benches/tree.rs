use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

use libbptree::*;


pub fn bench_large_tree(c: &mut Criterion) {
    let mut group = c.benchmark_group("Large Tree");

    group.bench_function("Order 2", |b| b.iter(|| 
        {
            let mut tree = BPlusTree::new(2);
            for i in 0..1024*1024 {
                tree.insert(i, i);
            }
        }
    ));

    group.bench_function("Order 3", |b| b.iter(|| 
        {
            let mut tree = BPlusTree::new(3);
            for i in 0..1024*1024 {
                tree.insert(i, i);
            }
        }
    ));

    group.bench_function("Order 4", |b| b.iter(|| 
        {
            let mut tree = BPlusTree::new(4);
            for i in 0..1024*1024 {
                tree.insert(i, i);
            }
        }
    ));

    group.bench_function("Order 8", |b| b.iter(|| 
        {
            let mut tree = BPlusTree::new(8);
            for i in 0..1024*1024 {
                tree.insert(i, i);
            }
        }
    ));

    group.bench_function("Order 12", |b| b.iter(|| 
        {
            let mut tree = BPlusTree::new(12);
            for i in 0..1024*1024 {
                tree.insert(i, i);
            }
        }
    ));
        

}



criterion_group!(name = add_locs_large;
    config = Criterion::default().measurement_time(std::time::Duration::from_secs(60));
    targets = bench_large_tree);

// criterion_main!(add_locs, add_locs_large);
criterion_main!(add_locs_large);
