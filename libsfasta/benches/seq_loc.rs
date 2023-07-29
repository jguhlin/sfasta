use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};

use libsfasta::datatypes::seq_loc::*;

use rand::Rng;

use std::io::Cursor;
use std::sync::Arc;

pub fn benchmark_add_locs(c: &mut Criterion) {
    
    let loc = Loc::Loc(0, 0, 128);

    let mut seqlocs = SeqLocs::default();
    let locs = vec![loc.clone(); 128];

    c.bench_with_input(BenchmarkId::new("add_locs", 128), &locs, |b, s| {
        b.iter(|| seqlocs.add_locs(&s))
    });

    let mut seqlocs = SeqLocs::default();
    let locs = vec![loc.clone(); 256];

    c.bench_with_input(BenchmarkId::new("add_locs", 256), &locs, |b, s| {
        b.iter(|| seqlocs.add_locs(&s))
    });

    let mut seqlocs = SeqLocs::default();
    let locs = vec![loc.clone(); 512];

    c.bench_with_input(BenchmarkId::new("add_locs", 512), &locs, |b, s| {
        b.iter(|| seqlocs.add_locs(&s))
    });

    let mut seqlocs = SeqLocs::default();
    let locs = vec![loc.clone(); 1024];

    c.bench_with_input(BenchmarkId::new("add_locs", 1024), &locs, |b, s| {
        b.iter(|| seqlocs.add_locs(&s))
    });
   
}

fn benchmark_add_locs_large(c: &mut Criterion) {
    let loc = Loc::Loc(0, 0, 128);

    let mut seqlocs = SeqLocs::default();
    let locs = vec![loc; 1024];

    c.bench_with_input(BenchmarkId::new("add_locs_large", 1024*1024*8), &locs, |b, s| {
        b.iter(|| 
            for _ in 0..1024*8 {
                seqlocs.add_locs(&s);
            }
        )
    });

    let mut seqlocs = SeqLocs::default();

    c.bench_with_input(BenchmarkId::new("add_locs_large", 1024*1024*16), &locs, |b, s| {
        b.iter(|| 
            for _ in 0..1024*16 {
                seqlocs.add_locs(&s);
            }
        )
    });

    let mut seqlocs = SeqLocs::default();

    c.bench_with_input(BenchmarkId::new("add_locs_large", 1024*1024*32), &locs, |b, s| {
        b.iter(|| 
            for _ in 0..1024*32 {
                seqlocs.add_locs(&s);
            }
        )
    });
}

criterion_group!(name = add_locs;
    config = Criterion::default(); //.measurement_time(std::time::Duration::from_secs(90));
    targets = benchmark_add_locs);

criterion_group!(name = add_locs_large;
    config = Criterion::default().measurement_time(std::time::Duration::from_secs(30));
    targets = benchmark_add_locs_large);

criterion_main!(add_locs, add_locs_large);
