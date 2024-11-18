use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};
use kanal::bounded;
use rand::Rng;
use std::thread;
use std::time::Duration;

use ringbuf::{traits::{Consumer, Producer, Split}, HeapRb};

fn generate_random_vec(size: usize) -> Vec<f32> {
    let mut rng = rand::thread_rng();
    (0..size).map(|_| rng.gen::<f32>()).collect()
}

fn benchmark_ringbuf(c: &mut Criterion) {
    let mut group = c.benchmark_group("channel_comparison");
    let sizes = [100, 1000, 10000];

    for size in sizes {
        group.bench_with_input(BenchmarkId::new("ringbuf", size), &size, |b, &size| {
            b.iter(|| {
                let rb = HeapRb::<Vec<f32>>::new(32);
                let (mut prod, mut cons) = rb.split();
                
                let producer_data = generate_random_vec(size);
                
                let producer = thread::spawn(move || {
                    prod.try_push(producer_data).unwrap();
                });

                let consumer = thread::spawn(move || {
                    while let Some(data) = cons.try_pop() {
                        criterion::black_box(data);
                    }
                });

                producer.join().unwrap();
                consumer.join().unwrap();
            });
        });

        group.bench_with_input(BenchmarkId::new("kanal", size), &size, |b, &size| {
            b.iter(|| {
                let (tx, rx) = bounded(32);
                
                let producer_data = generate_random_vec(size);
                
                let producer = thread::spawn(move || {
                    tx.send(producer_data).unwrap();
                });

                let consumer = thread::spawn(move || {
                    while let Ok(data) = rx.recv() {
                        criterion::black_box(data);
                    }
                });

                producer.join().unwrap();
                consumer.join().unwrap();
            });
        });
    }

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().measurement_time(Duration::from_secs(10));
    targets = benchmark_ringbuf
}
criterion_main!(benches); 