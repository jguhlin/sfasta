use criterion::{black_box, criterion_group, criterion_main, Criterion};

use libsfasta::prelude::*;

use std::fs::{metadata, File};
use std::io::Cursor;
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom};
use std::time::Duration;

fn create_sfasta(seq: &'static str, ct: CompressionType, index: bool) -> usize {
    let in_buf = BufReader::new(seq.as_bytes());
    let mut out_buf = Cursor::new(Vec::new());

    let mut converter = Converter::default().with_threads(16).with_block_size(32 * 1024).with_compression_type(ct);
    if !index {
        converter = converter.without_index();
    }

    converter.convert_fasta(
        in_buf,
        &mut out_buf,
    );

    out_buf.into_inner().len()
}

fn bench_conversion(c: &mut Criterion) {
    let seq = include_str!("../test_data/test_sequence_conversion.fasta");

    let zstd = create_sfasta(seq, CompressionType::ZSTD, true);
    let xz = create_sfasta(seq, CompressionType::XZ, true);
    let lz4 = create_sfasta(seq, CompressionType::LZ4, true);
    let brotli = create_sfasta(seq, CompressionType::BROTLI, true);

    println!("Uncompressed Size (no index): {}", seq.len());
    println!(
        "With Index, sizes: ZSTD: {} XZ: {} LZ4: {} BROTLI: {}",
        zstd, xz, lz4, brotli
    );

    let mut group = c.benchmark_group("Compression Types");
    group.bench_function("create_sfasta zstd index", |b| {
        b.iter_with_large_drop(|| create_sfasta(black_box(seq), CompressionType::ZSTD, true))
    });
    group.bench_function("create_sfasta zstd noindex", |b| {
        b.iter_with_large_drop(|| create_sfasta(black_box(seq), CompressionType::ZSTD, false))
    });
    group.bench_function("create_sfasta xz index", |b| {
        b.iter_with_large_drop(|| create_sfasta(black_box(seq), CompressionType::XZ, true))
    });
    group.bench_function("create_sfasta xz noindex", |b| {
        b.iter_with_large_drop(|| create_sfasta(black_box(seq), CompressionType::XZ, false))
    });
    group.bench_function("create_sfasta lz4 index", |b| {
        b.iter_with_large_drop(|| create_sfasta(black_box(seq), CompressionType::LZ4, true))
    });
    group.bench_function("create_sfasta lz4 noindex", |b| {
        b.iter_with_large_drop(|| create_sfasta(black_box(seq), CompressionType::LZ4, false))
    });
    group.bench_function("create_sfasta brotli index", |b| {
        b.iter_with_large_drop(|| create_sfasta(black_box(seq), CompressionType::BROTLI, true))
    });
    group.bench_function("create_sfasta brotli noindex", |b| {
        b.iter_with_large_drop(|| create_sfasta(black_box(seq), CompressionType::BROTLI, false))
    });
}

criterion_group! {
    name = conversion_benchmarks;
    config = Criterion::default().measurement_time(Duration::from_secs(64)).sample_size(250);
    targets = bench_conversion
}

criterion_main!(conversion_benchmarks);
