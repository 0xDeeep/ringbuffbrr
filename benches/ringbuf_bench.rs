use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput, BenchmarkId};
use ringbuffbrr::channel;
use std::time::Duration;

fn benchmark_single_thread(c: &mut Criterion) {
    let mut group = c.benchmark_group("single_thread");
    
    for size in [64, 256, 1024, 4096, 16384].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(size),
            size,
            |b, &size| {
                let (producer, consumer) = channel::<u64>(size);
                let mut counter = 0u64;
                
                b.iter(|| {
                    for _ in 0..1000 {
                        if !producer.push(counter) {
                            consumer.pop();
                            producer.push(counter);
                        }
                        counter += 1;
                    }
                    black_box(&producer);
                    black_box(&consumer);
                });
            }
        );
    }
    group.finish();
}

fn benchmark_spsc(c: &mut Criterion) {
    let mut group = c.benchmark_group("spsc");
    group.measurement_time(Duration::from_secs(10));
    
    for size in [256, 1024, 4096, 16384].iter() {
        group.throughput(Throughput::Elements(1));
        group.bench_with_input(
            BenchmarkId::from_parameter(size), 
            size,
            |b, &size| {
                b.iter_custom(|iters| {
                    let (tx, rx) = channel::<u64>(size);
                    
                    let start = std::time::Instant::now();
                    
                    let producer_thread = std::thread::spawn(move || {
                        for i in 0..iters {
                            while !tx.push(i) {
                                std::hint::spin_loop();
                            }
                        }
                    });
                    
                    let consumer_thread = std::thread::spawn(move || {
                        for _ in 0..iters {
                            while rx.pop().is_none() {
                                std::hint::spin_loop();
                            }
                        }
                    });
                    
                    producer_thread.join().unwrap();
                    consumer_thread.join().unwrap();
                    
                    start.elapsed()
                });
            }
        );
    }
    group.finish();
}

fn benchmark_batch_ops(c: &mut Criterion) {
    let mut group = c.benchmark_group("batch_operations");
    
    let (producer, consumer) = channel::<u64>(4096);
    let batch_sizes = [8, 16, 32, 64, 128];
    
    for batch_size in batch_sizes.iter() {
        group.throughput(Throughput::Elements(*batch_size as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            batch_size,
            |b, &batch_size| {
                let data: Vec<u64> = (0..batch_size as u64).collect();
                let mut out = vec![0u64; batch_size];
                
                b.iter(|| {
                    producer.push_slice(&data);
                    consumer.pop_slice(&mut out);
                    black_box(&out);
                });
            }
        );
    }
    group.finish();
}

// Compare with other implementations
fn benchmark_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("comparison");
    let size = 1024;
    let iterations = 10000;
    
    // Our implementation
    group.bench_function("fast_ringbuf", |b| {
        let (producer, consumer) = channel::<u64>(size);
        let mut counter = 0u64;
        
        b.iter(|| {
            for _ in 0..iterations {
                if !producer.push(counter) {
                    consumer.pop();
                    producer.push(counter);
                }
                counter += 1;
            }
        });
    });
    
    // Crossbeam channel
    group.bench_function("crossbeam", |b| {
        use crossbeam_channel::bounded;
        let (tx, rx) = bounded::<u64>(size);
        let mut counter = 0u64;
        
        b.iter(|| {
            for _ in 0..iterations {
                if tx.try_send(counter).is_err() {
                    rx.try_recv().ok();
                    tx.try_send(counter).ok();
                }
                counter += 1;
            }
        });
    });
    
    // Standard library MPSC
    group.bench_function("std_mpsc", |b| {
        use std::sync::mpsc::sync_channel;
        let (tx, rx) = sync_channel::<u64>(size);
        let mut counter = 0u64;
        
        b.iter(|| {
            for _ in 0..iterations {
                if tx.try_send(counter).is_err() {
                    rx.try_recv().ok();
                    tx.try_send(counter).ok();
                }
                counter += 1;
            }
        });
    });
    
    group.finish();
}

criterion_group!(
    benches,
    benchmark_single_thread,
    benchmark_spsc,
    benchmark_batch_ops,
    benchmark_comparison
);

criterion_main!(benches);