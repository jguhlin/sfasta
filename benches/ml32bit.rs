use criterion::{black_box, criterion_group, criterion_main, Criterion};

use libsfasta::masking::ml32bit::*;

fn criterion_benchmark(c: &mut Criterion) {
    let seq = include_str!("../test_data/Erow_sample.fasta");
    // Remove whitespace
    let seq = seq.split_whitespace().collect::<String>();

    c.bench_function("get_masking_ranges", |b| {
        b.iter(|| get_masking_ranges(black_box(seq.as_bytes())))
    });

    let ranges = get_masking_ranges(seq.as_bytes());
    c.bench_function("convert ranges to ml32bit", |b| {
        b.iter(|| convert_ranges_to_ml32bit(black_box(&ranges)))
    });

    let ml32bit = convert_ranges_to_ml32bit(&ranges);

    c.bench_function("pad_commands_to_u32", |b| {
        b.iter(|| pad_commands_to_u32(black_box(&ml32bit)))
    });

    let ml32bit_padded = pad_commands_to_u32(&ml32bit);

    c.bench_function("convert_commands_to_u32", |b| {
        b.iter(|| convert_commands_to_u32(black_box(&ml32bit_padded)))
    });

    let u32s = convert_commands_to_u32(&ml32bit_padded);

    c.bench_function("convert_u32_to_commands", |b| {
        b.iter(|| convert_u32_to_commands(black_box(&u32s)))
    });
    // let commands = convert_u32_to_commands(&u32s);

    // TODO:
    // let mut myseq = seq.clone();
    // c.bench_function("mask_sequences" , |b| b.iter(|| mask_sequence(black_box(&commands.clone()), black_box(&mut myseq.as_bytes()))));

    // c.bench_function("convert_uniprot", |b| b.iter(|| convert_uniprot()));
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
