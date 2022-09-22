use criterion::{black_box, criterion_group, criterion_main, Criterion};

use libsfasta::masking::ml32bit::*;

fn criterion_benchmark(c: &mut Criterion) {
    let seq = include_str!("../test_data/Erow_sample.fasta");
    // Remove whitespace
    let seq = seq.split_whitespace().collect::<String>();

    let mut group = c.benchmark_group("Get Masking Ranges");
    group.throughput(criterion::Throughput::Bytes(seq.len() as u64));
    group.bench_function("get_masking_ranges", |b| {
        b.iter(|| get_masking_ranges(black_box(seq.as_bytes())))
    });
    group.bench_function("get_masking_ranges_previous", |b| {
        b.iter(|| get_masking_ranges_previous(black_box(seq.as_bytes())))
    });
    group.finish();

    let ranges = get_masking_ranges(seq.as_bytes());

    let mut group = c.benchmark_group("Convert Ranges to ml32bit");
    group.throughput(criterion::Throughput::Bytes(
        (std::mem::size_of::<usize>() * 2 * ranges.len()) as u64,
    ));
    group.bench_function("convert ranges to ml32bit", |b| {
        b.iter(|| convert_ranges_to_ml32bit(black_box(&ranges)))
    });
    group.finish();

    let ml32bit = convert_ranges_to_ml32bit(&ranges);

    let mut group = c.benchmark_group("Pad Commands");
    group.throughput(criterion::Throughput::Bytes(
        (std::mem::size_of::<Ml32bit>() * ml32bit.len()) as u64,
    ));
    group.bench_function("pad_commands_to_u32", |b| {
        b.iter(|| pad_commands_to_u32(black_box(&ml32bit)))
    });
    group.finish();

    let ml32bit_padded = pad_commands_to_u32(&ml32bit);

    let mut group = c.benchmark_group("Convert Commands to u32");
    group.throughput(criterion::Throughput::Bytes(
        (std::mem::size_of::<Ml32bit>() * ml32bit.len()) as u64,
    ));
    group.bench_function("convert_commands_to_u32", |b| {
        b.iter(|| convert_commands_to_u32(black_box(&ml32bit_padded)))
    });
    group.bench_function("convert_commands_to_u32_uncompressed", |b| {
        b.iter(|| convert_commands_to_u32_uncompressed(black_box(&ml32bit_padded)))
    });
    // group.bench_function("convert_commands_to_u32_orig", |b| {
    //b.iter(|| convert_commands_to_u32_orig(black_box(&ml32bit_padded)))
    //});
    group.finish();

    let u32s = convert_commands_to_u32(&ml32bit_padded);

    let mut group = c.benchmark_group("Convert u32 to Commands");
    group.throughput(criterion::Throughput::Bytes(
        (std::mem::size_of::<u32>() * u32s.len()) as u64,
    ));
    group.bench_function("convert_u32_to_commands", |b| {
        b.iter(|| convert_u32_to_commands(black_box(&u32s)))
    });
    group.bench_function("convert_u32_to_commands_orig", |b| {
        b.iter(|| convert_u32_to_commands2(black_box(&u32s)))
    });
    group.finish();

    let commands = convert_u32_to_commands(&u32s);

    let mut myseq = seq.as_bytes().to_ascii_lowercase();

    let mut group = c.benchmark_group("Mask Sequence");
    group.throughput(criterion::Throughput::Bytes(
        (seq.len() + std::mem::size_of::<Ml32bit>() * commands.len()) as u64,
    ));
    group.bench_function("mask_sequence", |b| {
        b.iter(|| mask_sequence(black_box(&commands), black_box(&mut myseq)))
    });

    // let mut group = c.benchmark_group("Mask Sequences");
    // group.throughput(criterion::Throughput::Bytes((seq.len() + std::mem::size_of::<u32>() * u32s.len()) as u64));
    // let mut myseq = seq.clone();
    // group.bench_function("mask_sequences" , |b| b.iter(|| mask_sequence(black_box(&commands.clone()), black_box(&mut myseq.as_bytes()))));

    // c.bench_function("convert_uniprot", |b| b.iter(|| convert_uniprot()));
}

criterion_group! {
    name=ml32bit;
    config = Criterion::default().significance_level(0.05).sample_size(100000).measurement_time(std::time::Duration::from_secs(20));
    targets=criterion_benchmark
}

criterion_main!(ml32bit);
