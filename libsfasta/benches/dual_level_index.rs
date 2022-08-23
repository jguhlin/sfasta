use criterion::{black_box, criterion_group, criterion_main, Criterion};

use libsfasta::dual_level_index::*;

use rand::Rng;
use std::io::Cursor;

pub fn build_dual_index(data: Vec<(String, u32)>) {
    let mut out_buf: Cursor<Vec<u8>> = Cursor::new(Vec::new());
    let mut di = DualIndexBuilder::new();

    for (s, i) in data {
        di.add(s, i);
    }

    let mut writer: DualIndexWriter = di.into();
    writer.write_to_buffer(&mut out_buf);
}

pub fn di_benchmark(c: &mut Criterion) {
    //c.measurement_time(std::time::Duration::from_secs(90));

    let mut rng = rand::thread_rng();
    let mut data = Vec::with_capacity(1_024 * 1_024);

    for i in (0_u64..10000) {
        data.push((i.to_string(), (i * 2) as u32));
    }

    data.push(("Max".to_string(), std::u32::MAX));

    for i in (0_u64..10000).step_by(2) {
        data.push((i.to_string(), rng.gen::<u32>()));
    }

    data.push(("Max".to_string(), std::u32::MAX));

    for i in (0_u64..1000).step_by(2) {
        data.push((i.to_string(), rng.gen::<u32>()));
    }

    c.bench_function("dual index builder", |b| {
        b.iter(|| build_dual_index(black_box(data.clone())))
    });
}

criterion_group!(name = dual_level_index;
    config = Criterion::default().measurement_time(std::time::Duration::from_secs(90));
    targets = di_benchmark);
criterion_main!(dual_level_index);
