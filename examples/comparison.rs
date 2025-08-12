use ringbuffbrr::channel;
use std::time::{Instant, Duration};
use std::thread;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const WARMUP_ITERATIONS: usize = 1_000_000;

fn main() {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║        RING BUFFER PERFORMANCE COMPARISON SUITE             ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    
    print_system_info();
    
    println!("\n🔥 WARMUP PHASE");
    println!("════════════════════════════════════════════════════════════════");
    warmup();
    
    println!("\n📊 SINGLE-THREADED COMPARISON");
    println!("════════════════════════════════════════════════════════════════");
    single_threaded_comparison();
    
    println!("\n📊 MULTI-THREADED THROUGHPUT COMPARISON");
    println!("════════════════════════════════════════════════════════════════");
    multi_threaded_comparison();
    
    println!("\n📊 LATENCY COMPARISON");
    println!("════════════════════════════════════════════════════════════════");
    latency_comparison();
    
    println!("\n📊 BATCH OPERATIONS COMPARISON");
    println!("════════════════════════════════════════════════════════════════");
    batch_comparison();
    
    println!("\n🏆 FINAL VERDICT");
    println!("════════════════════════════════════════════════════════════════");
    final_verdict();
}

fn print_system_info() {
    println!("\n📋 System Information:");
    println!("─────────────────────────────────────────────────────────────────");
    
    #[cfg(target_arch = "x86_64")]
    {
        println!("Architecture: x86_64 (with AVX2/prefetch support)");
    }
    
    #[cfg(not(target_arch = "x86_64"))]
    println!("Architecture: {} (non-x86_64)", std::env::consts::ARCH);
}

fn warmup() {
    let (producer, consumer) = channel::<u64>(1024);
    
    print!("Warming up CPU caches...");
    
    // Warmup to get CPU to max frequency and warm up caches
    for i in 0..WARMUP_ITERATIONS {
        if !producer.push(i as u64) {
            consumer.pop();
            producer.push(i as u64);
        }
    }
    
    // Drain
    while consumer.pop().is_some() {}
    
    println!(" ✅ Complete");
}

fn single_threaded_comparison() {
    let iterations = 100_000_000;
    let buffer_size = 4096;
    
    println!("\nSingle-threaded performance (100M operations):");
    println!("┌─────────────────────┬──────────────┬──────────────┬──────────────┐");
    println!("│ Implementation      │  Time (ms)   │   Ops/sec    │   Relative   │");
    println!("├─────────────────────┼──────────────┼──────────────┼──────────────┤");
    
    // Test ringbuffbrr
    let (producer, consumer) = channel::<u64>(buffer_size);
    let start = Instant::now();
    for i in 0..iterations {
        if !producer.push(i) { 
            consumer.pop(); 
            producer.push(i); 
        }
    }
    let ringbuffbrr_time = start.elapsed();
    let ringbuffbrr_ops = iterations as f64 / ringbuffbrr_time.as_secs_f64();
    
    // Test crossbeam-channel
    use crossbeam_channel::bounded;
    let (tx, rx) = bounded(buffer_size);
    let start = Instant::now();
    for i in 0..iterations {
        if tx.try_send(i).is_err() {
            rx.try_recv().ok();
            tx.try_send(i).ok();
        }
    }
    let crossbeam_time = start.elapsed();
    let crossbeam_ops = iterations as f64 / crossbeam_time.as_secs_f64();
    
    // Test std::sync::mpsc
    use std::sync::mpsc;
    let (tx, rx) = mpsc::sync_channel(buffer_size);
    let start = Instant::now();
    for i in 0..iterations {
        if tx.try_send(i).is_err() {
            rx.try_recv().ok();
            tx.try_send(i).ok();
        }
    }
    let mpsc_time = start.elapsed();
    let mpsc_ops = iterations as f64 / mpsc_time.as_secs_f64();
    
    let fastest = ringbuffbrr_ops.max(crossbeam_ops).max(mpsc_ops);
    
    println!("│ ringbuffbrr         │{:12.0} │{:12.0} M │{:12.1}x │", 
             ringbuffbrr_time.as_millis(), ringbuffbrr_ops / 1_000_000.0, ringbuffbrr_ops / fastest);
    println!("│ crossbeam-channel   │{:12.0} │{:12.0} M │{:12.1}x │", 
             crossbeam_time.as_millis(), crossbeam_ops / 1_000_000.0, crossbeam_ops / fastest);
    println!("│ std::sync::mpsc     │{:12.0} │{:12.0} M │{:12.1}x │", 
             mpsc_time.as_millis(), mpsc_ops / 1_000_000.0, mpsc_ops / fastest);
    println!("└─────────────────────┴──────────────┴──────────────┴──────────────┘");
    
    let winner = if ringbuffbrr_ops > crossbeam_ops && ringbuffbrr_ops > mpsc_ops {
        "ringbuffbrr"
    } else if crossbeam_ops > mpsc_ops {
        "crossbeam-channel"
    } else {
        "std::sync::mpsc"
    };
    
    println!("\n🏆 Single-threaded winner: {} ({:.1}x faster than slowest)", 
             winner, fastest / ringbuffbrr_ops.min(crossbeam_ops).min(mpsc_ops));
}

fn multi_threaded_comparison() {
    let test_duration = Duration::from_secs(2);
    let buffer_size = 4096;
    
    println!("\nMulti-threaded throughput (2 seconds per test):");
    println!("┌─────────────────────┬──────────────┬──────────────┬──────────────┐");
    println!("│ Implementation      │  Throughput  │   Msgs/sec   │   Relative   │");
    println!("├─────────────────────┼──────────────┼──────────────┼──────────────┤");
    
    // Test ringbuffbrr
    let ringbuffbrr_throughput = test_multi_threaded_ringbuffbrr(buffer_size, test_duration);
    
    // Test crossbeam-channel
    let crossbeam_throughput = test_multi_threaded_crossbeam(buffer_size, test_duration);
    
    // Test std::sync::mpsc
    let mpsc_throughput = test_multi_threaded_mpsc(buffer_size, test_duration);
    
    let fastest = ringbuffbrr_throughput.max(crossbeam_throughput).max(mpsc_throughput);
    
    println!("│ ringbuffbrr         │{:12.2} M/s│{:12.0} │{:12.1}x │", 
             ringbuffbrr_throughput / 1_000_000.0, ringbuffbrr_throughput, ringbuffbrr_throughput / fastest);
    println!("│ crossbeam-channel   │{:12.2} M/s│{:12.0} │{:12.1}x │", 
             crossbeam_throughput / 1_000_000.0, crossbeam_throughput, crossbeam_throughput / fastest);
    println!("│ std::sync::mpsc     │{:12.2} M/s│{:12.0} │{:12.1}x │", 
             mpsc_throughput / 1_000_000.0, mpsc_throughput, mpsc_throughput / fastest);
    println!("└─────────────────────┴──────────────┴──────────────┴──────────────┘");
    
    let winner = if ringbuffbrr_throughput > crossbeam_throughput && ringbuffbrr_throughput > mpsc_throughput {
        "ringbuffbrr"
    } else if crossbeam_throughput > mpsc_throughput {
        "crossbeam-channel"
    } else {
        "std::sync::mpsc"
    };
    
    println!("\n🏆 Multi-threaded winner: {} ({:.1}x faster than slowest)", 
             winner, fastest / ringbuffbrr_throughput.min(crossbeam_throughput).min(mpsc_throughput));
}

fn test_multi_threaded_ringbuffbrr(buffer_size: usize, duration: Duration) -> f64 {
    let (producer, consumer) = channel::<u64>(buffer_size);
    let running = Arc::new(AtomicBool::new(true));
    let running_producer = running.clone();
    let running_consumer = running.clone();
    
    let producer_handle = thread::spawn(move || {
        let mut count = 0u64;
        let mut value = 0u64;
        
        while running_producer.load(Ordering::Relaxed) {
            if producer.push(value) {
                value = value.wrapping_add(1);
                count += 1;
            } else {
                std::hint::spin_loop();
            }
        }
        count
    });
    
    let consumer_handle = thread::spawn(move || {
        let mut count = 0u64;
        
        while running_consumer.load(Ordering::Relaxed) {
            if consumer.pop().is_some() {
                count += 1;
            } else {
                std::hint::spin_loop();
            }
        }
        count
    });
    
    thread::sleep(duration);
    running.store(false, Ordering::Relaxed);
    
    let _produced = producer_handle.join().unwrap();
    let consumed = consumer_handle.join().unwrap();
    
    consumed as f64 / duration.as_secs_f64()
}

fn test_multi_threaded_crossbeam(buffer_size: usize, duration: Duration) -> f64 {
    use crossbeam_channel::bounded;
    let (tx, rx) = bounded(buffer_size);
    let running = Arc::new(AtomicBool::new(true));
    let running_producer = running.clone();
    let running_consumer = running.clone();
    
    let producer_handle = thread::spawn(move || {
        let mut count = 0u64;
        let mut value = 0u64;
        
        while running_producer.load(Ordering::Relaxed) {
            if tx.try_send(value).is_ok() {
                value = value.wrapping_add(1);
                count += 1;
            } else {
                std::hint::spin_loop();
            }
        }
        count
    });
    
    let consumer_handle = thread::spawn(move || {
        let mut count = 0u64;
        
        while running_consumer.load(Ordering::Relaxed) {
            if rx.try_recv().is_ok() {
                count += 1;
            } else {
                std::hint::spin_loop();
            }
        }
        count
    });
    
    thread::sleep(duration);
    running.store(false, Ordering::Relaxed);
    
    let _produced = producer_handle.join().unwrap();
    let consumed = consumer_handle.join().unwrap();
    
    consumed as f64 / duration.as_secs_f64()
}

fn test_multi_threaded_mpsc(buffer_size: usize, duration: Duration) -> f64 {
    use std::sync::mpsc;
    let (tx, rx) = mpsc::sync_channel(buffer_size);
    let running = Arc::new(AtomicBool::new(true));
    let running_producer = running.clone();
    let running_consumer = running.clone();
    
    let producer_handle = thread::spawn(move || {
        let mut count = 0u64;
        let mut value = 0u64;
        
        while running_producer.load(Ordering::Relaxed) {
            if tx.try_send(value).is_ok() {
                value = value.wrapping_add(1);
                count += 1;
            } else {
                std::hint::spin_loop();
            }
        }
        count
    });
    
    let consumer_handle = thread::spawn(move || {
        let mut count = 0u64;
        
        while running_consumer.load(Ordering::Relaxed) {
            if rx.try_recv().is_ok() {
                count += 1;
            } else {
                std::hint::spin_loop();
            }
        }
        count
    });
    
    thread::sleep(duration);
    running.store(false, Ordering::Relaxed);
    
    let _produced = producer_handle.join().unwrap();
    let consumed = consumer_handle.join().unwrap();
    
    consumed as f64 / duration.as_secs_f64()
}

fn latency_comparison() {
    let iterations = 1_000_000;
    let buffer_size = 1024;
    
    println!("\nRound-trip latency comparison (1M operations):");
    println!("┌─────────────────────┬──────────┬──────────┬──────────┬──────────┐");
    println!("│ Implementation      │   p50    │   p99    │  p99.9   │  p99.99  │");
    println!("├─────────────────────┼──────────┼──────────┼──────────┼──────────┤");
    
    // Test ringbuffbrr latency
    let ringbuffbrr_latencies = measure_latency_ringbuffbrr(buffer_size, iterations);
    let (p50, p99, p999, p9999) = calculate_percentiles(&ringbuffbrr_latencies);
    println!("│ ringbuffbrr         │{:8.0} ns│{:8.0} ns│{:8.0} ns│{:8.0} ns│", p50, p99, p999, p9999);
    
    // Test crossbeam latency
    let crossbeam_latencies = measure_latency_crossbeam(buffer_size, iterations);
    let (p50, p99, p999, p9999) = calculate_percentiles(&crossbeam_latencies);
    println!("│ crossbeam-channel   │{:8.0} ns│{:8.0} ns│{:8.0} ns│{:8.0} ns│", p50, p99, p999, p9999);
    
    // Test mpsc latency
    let mpsc_latencies = measure_latency_mpsc(buffer_size, iterations);
    let (p50, p99, p999, p9999) = calculate_percentiles(&mpsc_latencies);
    println!("│ std::sync::mpsc     │{:8.0} ns│{:8.0} ns│{:8.0} ns│{:8.0} ns│", p50, p99, p999, p9999);
    
    println!("└─────────────────────┴──────────┴──────────┴──────────┴──────────┘");
}

fn measure_latency_ringbuffbrr(buffer_size: usize, iterations: usize) -> Vec<f64> {
    let (producer, consumer) = channel::<u64>(buffer_size);
    let mut latencies = Vec::with_capacity(iterations);
    
    // Pre-fill buffer halfway
    for i in 0..buffer_size/2 {
        producer.push(i as u64);
    }
    
    for i in 0..iterations {
        let start = Instant::now();
        if !producer.push(i as u64) {
            consumer.pop();
            producer.push(i as u64);
        }
        consumer.pop();
        let elapsed = start.elapsed();
        latencies.push(elapsed.as_nanos() as f64);
    }
    
    latencies
}

fn measure_latency_crossbeam(buffer_size: usize, iterations: usize) -> Vec<f64> {
    use crossbeam_channel::bounded;
    let (tx, rx) = bounded(buffer_size);
    let mut latencies = Vec::with_capacity(iterations);
    
    // Pre-fill buffer halfway
    for i in 0..buffer_size/2 {
        tx.try_send(i).ok();
    }
    
    for i in 0..iterations {
        let start = Instant::now();
        if tx.try_send(i).is_err() {
            rx.try_recv().ok();
            tx.try_send(i).ok();
        }
        rx.try_recv().ok();
        let elapsed = start.elapsed();
        latencies.push(elapsed.as_nanos() as f64);
    }
    
    latencies
}

fn measure_latency_mpsc(buffer_size: usize, iterations: usize) -> Vec<f64> {
    use std::sync::mpsc;
    let (tx, rx) = mpsc::sync_channel(buffer_size);
    let mut latencies = Vec::with_capacity(iterations);
    
    // Pre-fill buffer halfway
    for i in 0..buffer_size/2 {
        tx.try_send(i).ok();
    }
    
    for i in 0..iterations {
        let start = Instant::now();
        if tx.try_send(i).is_err() {
            rx.try_recv().ok();
            tx.try_send(i).ok();
        }
        rx.try_recv().ok();
        let elapsed = start.elapsed();
        latencies.push(elapsed.as_nanos() as f64);
    }
    
    latencies
}

fn calculate_percentiles(latencies: &[f64]) -> (f64, f64, f64, f64) {
    let mut sorted = latencies.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    
    let len = sorted.len();
    let p50 = sorted[len * 50 / 100];
    let p99 = sorted[len * 99 / 100];
    let p999 = sorted[len * 999 / 1000];
    let p9999 = sorted[len * 9999 / 10000];
    
    (p50, p99, p999, p9999)
}

fn batch_comparison() {
    let batch_sizes = [1, 4, 16, 64, 256];
    let buffer_size = 4096;
    let total_operations = 10_000_000;
    
    println!("\nBatch operations comparison:");
    println!("┌────────────┬─────────────────────┬─────────────────────┬─────────────────────┐");
    println!("│ Batch Size │     ringbuffbrr     │   crossbeam-channel │   std::sync::mpsc   │");
    println!("├────────────┼─────────────────────┼─────────────────────┼─────────────────────┤");
    
    for batch_size in batch_sizes {
        let iterations = total_operations / batch_size;
        
        // Test ringbuffbrr
        let ringbuffbrr_time = test_batch_ringbuffbrr(buffer_size, batch_size, iterations);
        let ringbuffbrr_rate = total_operations as f64 / ringbuffbrr_time.as_secs_f64();
        
        // Test crossbeam
        let crossbeam_time = test_batch_crossbeam(buffer_size, batch_size, iterations);
        let crossbeam_rate = total_operations as f64 / crossbeam_time.as_secs_f64();
        
        // Test mpsc
        let mpsc_time = test_batch_mpsc(buffer_size, batch_size, iterations);
        let mpsc_rate = total_operations as f64 / mpsc_time.as_secs_f64();
        
        println!("│{:10} │{:19.0} M │{:19.0} M │{:19.0} M │", 
                 batch_size, 
                 ringbuffbrr_rate / 1_000_000.0,
                 crossbeam_rate / 1_000_000.0,
                 mpsc_rate / 1_000_000.0);
    }
    
    println!("└────────────┴─────────────────────┴─────────────────────┴─────────────────────┘");
}

fn test_batch_ringbuffbrr(buffer_size: usize, batch_size: usize, iterations: usize) -> Duration {
    let (producer, consumer) = channel::<u64>(buffer_size);
    
    let start = Instant::now();
    for i in 0..iterations {
        for j in 0..batch_size {
            let value = (i * batch_size + j) as u64;
            if !producer.push(value) {
                consumer.pop();
                producer.push(value);
            }
        }
        for _ in 0..batch_size {
            consumer.pop();
        }
    }
    start.elapsed()
}

fn test_batch_crossbeam(buffer_size: usize, batch_size: usize, iterations: usize) -> Duration {
    use crossbeam_channel::bounded;
    let (tx, rx) = bounded(buffer_size);
    
    let start = Instant::now();
    for i in 0..iterations {
        for j in 0..batch_size {
            let value = (i * batch_size + j) as u64;
            if tx.try_send(value).is_err() {
                rx.try_recv().ok();
                tx.try_send(value).ok();
            }
        }
        for _ in 0..batch_size {
            rx.try_recv().ok();
        }
    }
    start.elapsed()
}

fn test_batch_mpsc(buffer_size: usize, batch_size: usize, iterations: usize) -> Duration {
    use std::sync::mpsc;
    let (tx, rx) = mpsc::sync_channel(buffer_size);
    
    let start = Instant::now();
    for i in 0..iterations {
        for j in 0..batch_size {
            let value = (i * batch_size + j) as u64;
            if tx.try_send(value).is_err() {
                rx.try_recv().ok();
                tx.try_send(value).ok();
            }
        }
        for _ in 0..batch_size {
            rx.try_recv().ok();
        }
    }
    start.elapsed()
}

fn final_verdict() {
    println!("\nOverall performance assessment:");
    println!("────────────────────────────────────────────────────────");
    
    // Quick summary benchmark
    let iterations = 10_000_000;
    let (producer, consumer) = channel::<u64>(4096);
    
    let start = Instant::now();
    for i in 0..iterations {
        if !producer.push(i) { 
            consumer.pop(); 
            producer.push(i); 
        }
    }
    let elapsed = start.elapsed();
    let ops_per_sec = iterations as f64 / elapsed.as_secs_f64();
    
    println!("ringbuffbrr final score: {:.1} M ops/sec", ops_per_sec / 1_000_000.0);
    
    let rating = match ops_per_sec {
        x if x > 500_000_000.0 => "🏆 WORLD CLASS - Approaching hardware limits!",
        x if x > 300_000_000.0 => "🥇 EXCEPTIONAL - Top 1% performance",
        x if x > 200_000_000.0 => "🥈 EXCELLENT - Production champion",
        x if x > 100_000_000.0 => "🥉 VERY GOOD - High performance",
        x if x > 50_000_000.0  => "✅ GOOD - Solid performance",
        _ => "⚠️ Needs optimization"
    };
    
    println!("Performance Rating: {}", rating);
}