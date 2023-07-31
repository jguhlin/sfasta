use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

use libsfasta::datatypes::seq_loc::*;

use rand::Rng;

use std::io::Cursor;
use std::sync::Arc;

#[derive(Default)]
struct OriginalFormatSeqLocs {
    data: Option<Vec<(u32, u32, u32)>>,
    total_locs: usize,
}

impl OriginalFormatSeqLocs {
    pub fn add_locs(&mut self, locs: &[(u32, u32, u32)]) -> (u64, u32) {
        if self.data.is_none() {
            self.data = Some(Vec::with_capacity(8192 * 64));
        }

        let start = self.total_locs;
        let len = locs.len();
        self.total_locs += len;
        self.data.as_mut().unwrap().extend_from_slice(locs);
        (start as u64, len as u32)
    }
}

#[derive(Clone, Copy)]
struct StructSeqLoc {
    block: u32,
    start: u32,
    len: u32,
}

#[derive(Default)]
struct StructLocSeqLocs {
    data: Option<Vec<StructSeqLoc>>,
    total_locs: usize,
}

impl StructLocSeqLocs {
    pub fn add_locs(&mut self, locs: &[StructSeqLoc]) -> (u64, u32) {
        if self.data.is_none() {
            self.data = Some(Vec::with_capacity(8192 * 64));
        }

        let start = self.total_locs;
        let len = locs.len();
        self.total_locs += len;
        self.data.as_mut().unwrap().extend_from_slice(locs);
        (start as u64, len as u32)
    }
}

#[derive(Clone, Copy)]
struct TupleStructLoc(u32, u32, u32);

#[derive(Default)]
struct TrupleStructLocSeqLocs {
    data: Option<Vec<TupleStructLoc>>,
    total_locs: usize,
}

impl TrupleStructLocSeqLocs {
    pub fn add_locs(&mut self, locs: &[TupleStructLoc]) -> (u64, u32) {
        if self.data.is_none() {
            self.data = Some(Vec::with_capacity(8192 * 64));
        }

        let start = self.total_locs;
        let len = locs.len();
        self.total_locs += len;
        self.data.as_mut().unwrap().extend_from_slice(locs);
        (start as u64, len as u32)
    }
}

pub fn benchmark_add_locs(c: &mut Criterion) {
    let loc = Loc::new(0, 0, 128);

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
    let loc = Loc::new(0, 0, 128);
    let locs = vec![loc; 1024];

    c.bench_with_input(
        BenchmarkId::new("add_locs_large", 1024 * 1024 * 8),
        &locs,
        |b, s| {
            b.iter(|| {
                let mut seqlocs = SeqLocs::default();
                for _ in 0..1024 * 8 {
                    seqlocs.add_locs(&s);
                }
            })
        },
    );

    c.bench_with_input(
        BenchmarkId::new("add_locs_large", 1024 * 1024 * 16),
        &locs,
        |b, s| {
            b.iter(|| {
                let mut seqlocs = SeqLocs::default();
                for _ in 0..1024 * 16 {
                    seqlocs.add_locs(&s);
                }
            })
        },
    );

    c.bench_with_input(
        BenchmarkId::new("add_locs_large", 1024 * 1024 * 32),
        &locs,
        |b, s| {
            b.iter(|| {
                let mut seqlocs = SeqLocs::default();
                for _ in 0..1024 * 32 {
                    seqlocs.add_locs(&s);
                }
            })
        },
    );

    let loc = StructSeqLoc {
        block: 0,
        start: 0,
        len: 128,
    };
    let locs = vec![loc; 1024];

    c.bench_with_input(
        BenchmarkId::new("add_locs_large_structformat", 1024 * 1024 * 32),
        &locs,
        |b, s| {
            b.iter(|| {
                let mut seqlocs = StructLocSeqLocs::default();
                for _ in 0..1024 * 32 {
                    seqlocs.add_locs(&s);
                }
            })
        },
    );

    let loc = (0, 0, 128);
    let locs = vec![loc; 1024];

    c.bench_with_input(
        BenchmarkId::new("add_locs_large_originalformat", 1024 * 1024 * 32),
        &locs,
        |b, s| {
            b.iter(|| {
                let mut seqlocs = OriginalFormatSeqLocs::default();
                for _ in 0..1024 * 32 {
                    seqlocs.add_locs(&s);
                }
            })
        },
    );

    let loc = TupleStructLoc(0, 0, 128);
    let locs = vec![loc; 1024];

    c.bench_with_input(
        BenchmarkId::new("add_locs_large_truplestructformat", 1024 * 1024 * 32),
        &locs,
        |b, s| {
            b.iter(|| {
                let mut seqlocs = TrupleStructLocSeqLocs::default();
                for _ in 0..1024 * 32 {
                    seqlocs.add_locs(&s);
                }
            })
        },
    );
}

// Disabled while we work on the rest...
/*criterion_group!(name = add_locs;
config = Criterion::default(); //.measurement_time(std::time::Duration::from_secs(90));
targets = benchmark_add_locs);*/

criterion_group!(name = add_locs_large;
    config = Criterion::default().measurement_time(std::time::Duration::from_secs(60));
    targets = benchmark_add_locs_large);

// criterion_main!(add_locs, add_locs_large);
criterion_main!(add_locs_large);
