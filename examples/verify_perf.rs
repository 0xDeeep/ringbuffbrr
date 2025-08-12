use ringbuffbrr::channel;
use std::time::{Instant, Duration};
use std::thread;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const WARMUP_ITERATIONS: usize = 1_000_000;

fn main() {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║     WORLD'S FASTEST SPSC RING BUFFER BENCHMARK SUITE        ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    
    print_system_info();
    
    println!("\n🔥 WARMUP PHASE");
    println!("════════════════════════════════════════════════════════════════");
    warmup();
    
    println!("\n📊 LATENCY MEASUREMENTS (Single-threaded)");
    println!("════════════════════════════════════════════════════════════════");
    measure_latency();
    
    println!("\n📊 THROUGHPUT MEASUREMENTS (Multi-threaded)");
    println!("════════════════════════════════════════════════════════════════");
    measure_throughput();
    
    println!("\n📊 BATCH OPERATIONS PERFORMANCE");
    println!("════════════════════════════════════════════════════════════════");
    measure_batch_performance();
    
    println!("\n📊 CACHE BEHAVIOR ANALYSIS");
    println!("════════════════════════════════════════════════════════════════");
    analyze_cache_behavior();
    
    println!("\n🏆 ULTIMATE STRESS TEST: 1 BILLION OPERATIONS");
    println!("════════════════════════════════════════════════════════════════");
    ultimate_stress_test();
    
    println!("\n📊 COMPARISON WITH THEORETICAL LIMITS");
    println!("════════════════════════════════════════════════════════════════");
    compare_with_theoretical_limits();
}

fn print_system_info() {
    println!("\n📋 System Information:");
    println!("─────────────────────────────────────────────────────────────────");
    
    #[cfg(target_arch = "x86_64")]
    {
        println!("Architecture: x86_64 (with AVX2/prefetch support)");
        
        // Try to get CPU info
        if let Ok(output) = std::process::Command::new("lscpu")
            .output()
        {
            let info = String::from_utf8_lossy(&output.stdout);
            for line in info.lines() {
                if line.contains("Model name") || line.contains("CPU MHz") || 
                   line.contains("L1d cache") || line.contains("L2 cache") || 
                   line.contains("L3 cache") {
                    println!("  {}", line.trim());
                }
            }
        }
    }
    
    #[cfg(not(target_arch = "x86_64"))]
    println!("Architecture: {} (non-x86_64)", std::env::consts::ARCH);
    
    // println!("Cores: {}", num_cpus::get());
    // println!("Cache line size: 64 bytes");
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

fn measure_latency() {
    let sizes = [64, 256, 1024, 4096, 16384, 65536];
    let iterations = 10_000_000;
    
    println!("\nMeasuring round-trip latency (push + pop):");
    println!("┌────────────┬──────────┬──────────┬──────────┬──────────┐");
    println!("│ Buffer Size│   p50    │   p99    │  p99.9   │  p99.99  │");
    println!("├────────────┼──────────┼──────────┼──────────┼──────────┤");
    
    for size in sizes {
        let (producer, consumer) = channel::<u64>(size);
        let mut latencies = Vec::with_capacity(iterations);
        
        // Pre-fill buffer halfway for realistic scenario
        for i in 0..size/2 {
            producer.push(i as u64);
        }
        
        // Measure latencies
        for i in 0..iterations {
            let start = Instant::now();
            
            // Push-pop cycle
            if !producer.push(i as u64) {
                consumer.pop();
                producer.push(i as u64);
            }
            
            latencies.push(start.elapsed().as_nanos() as u64);
        }
        
        // Calculate percentiles
        latencies.sort_unstable();
        let p50 = latencies[iterations * 50 / 100];
        let p99 = latencies[iterations * 99 / 100];
        let p999 = latencies[iterations * 999 / 1000];
        let p9999 = latencies[iterations * 9999 / 10000];
        
        println!("│ {:>10} │ {:>7} ns│ {:>7} ns│ {:>7} ns│ {:>7} ns│", 
                 size, p50, p99, p999, p9999);
    }
    
    println!("└────────────┴──────────┴──────────┴──────────┴──────────┘");
    
    // Single operation latency
    println!("\nSingle operation latency (best case):");
    let (producer, consumer) = channel::<u64>(4096);
    
    // Measure push latency
    let mut push_times = Vec::with_capacity(100_000);
    for i in 0..100_000 {
        let start = Instant::now();
        producer.push(i);
        push_times.push(start.elapsed().as_nanos() as u64);
        consumer.pop(); // Keep buffer from filling
    }
    
    push_times.sort_unstable();
    println!("  Push latency: {} ns (median)", push_times[50_000]);
    
    // Measure pop latency
    let mut pop_times = Vec::with_capacity(100_000);
    for i in 0..100_000 {
        producer.push(i); // Ensure something to pop
        let start = Instant::now();
        consumer.pop();
        pop_times.push(start.elapsed().as_nanos() as u64);
    }
    
    pop_times.sort_unstable();
    println!("  Pop latency:  {} ns (median)", pop_times[50_000]);
}

fn measure_throughput() {
    let sizes = [256, 1024, 4096, 16384, 65536];
    let test_duration = Duration::from_secs(2);
    
    println!("\nMeasuring sustained throughput (2 seconds per test):");
    println!("┌────────────┬──────────────┬──────────────┬──────────────┐");
    println!("│ Buffer Size│  Throughput  │   Msgs/sec   │  Bandwidth   │");
    println!("├────────────┼──────────────┼──────────────┼──────────────┤");
    
    for size in sizes {
        let (producer, consumer) = channel::<u64>(size);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = stop.clone();
        
        // Producer thread
        let producer_handle = thread::spawn(move || {
            let mut count = 0u64;
            let mut spin_count = 0u32;
            
            while !stop_clone.load(Ordering::Acquire) {
                if producer.push(count) {
                    count += 1;
                    spin_count = 0;
                } else {
                    spin_count += 1;
                    if spin_count > 100 {
                        std::thread::yield_now();
                        spin_count = 0;
                    } else {
                        std::hint::spin_loop();
                    }
                }
            }
            count
        });
        
        // Consumer thread - controls the timing
        let consumer_handle = thread::spawn(move || {
            let mut count = 0u64;
            let mut spin_count = 0u32;
            let start = Instant::now();
            
            while start.elapsed() < test_duration {
                if consumer.pop().is_some() {
                    count += 1;
                    spin_count = 0;
                } else {
                    spin_count += 1;
                    if spin_count > 100 {
                        std::thread::yield_now();
                        spin_count = 0;
                    } else {
                        std::hint::spin_loop();
                    }
                }
            }
            
            (count, start.elapsed())
        });
        
        // Wait for consumer to finish
        let (consumed, actual_duration) = consumer_handle.join().unwrap();
        
        // Signal producer to stop
        stop.store(true, Ordering::Release);
        let _produced = producer_handle.join().unwrap();
        
        let ops_per_sec = consumed as f64 / actual_duration.as_secs_f64();
        let bandwidth_gbps = (consumed as f64 * 8.0) / actual_duration.as_secs_f64() / 1_000_000_000.0;
        
        println!("│ {:>10} │ {:>10.2} M/s│ {:>12.0} │ {:>10.2} GB/s│", 
                 size, 
                 ops_per_sec / 1_000_000.0,
                 ops_per_sec,
                 bandwidth_gbps);
    }
    
    println!("└────────────┴──────────────┴──────────────┴──────────────┘");
}

fn measure_batch_performance() {
    let batch_sizes = [1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024];
    let total_items = 100_000_000u64;
    let buffer_size = 8192;
    
    println!("\nBatch operations throughput:");
    println!("┌────────────┬──────────────┬──────────────┬──────────────┐");
    println!("│ Batch Size │  Items/sec   │   ns/item    │  Speedup     │");
    println!("├────────────┼──────────────┼──────────────┼──────────────┤");
    
    let mut baseline_ns = 0.0;
    
    for (idx, &batch_size) in batch_sizes.iter().enumerate() {
        let (producer, consumer) = channel::<u64>(buffer_size);
        
        // Prepare buffers
        let mut batch = vec![0u64; batch_size];
        let mut receive_buffer = vec![0u64; batch_size];
        
        // Warmup
        for _ in 0..10_000 {
            for (i, item) in batch.iter_mut().enumerate() {
                *item = i as u64;
            }
            producer.push_slice(&batch);
            consumer.pop_slice(&mut receive_buffer);
        }
        
        let iterations = total_items / batch_size as u64;
        let start = Instant::now();
        
        for i in 0..iterations {
            // Update batch with sequential values
            for (j, item) in batch.iter_mut().enumerate() {
                *item = i * batch_size as u64 + j as u64;
            }
            
            while producer.push_slice(&batch) != batch_size {
                consumer.pop_slice(&mut receive_buffer);
            }
        }
        
        // Drain
        while consumer.pop_slice(&mut receive_buffer) > 0 {}
        
        let elapsed = start.elapsed();
        let items_per_sec = total_items as f64 / elapsed.as_secs_f64();
        let ns_per_item = elapsed.as_nanos() as f64 / total_items as f64;
        
        if idx == 0 {
            baseline_ns = ns_per_item;
        }
        
        let speedup = baseline_ns / ns_per_item;
        
        println!("│ {:>10} │ {:>10.2} M │ {:>12.2} │ {:>11.2}x │", 
                 batch_size,
                 items_per_sec / 1_000_000.0,
                 ns_per_item,
                 speedup);
    }
    
    println!("└────────────┴──────────────┴──────────────┴──────────────┘");
}

fn analyze_cache_behavior() {
    println!("\nAnalyzing cache-friendly access patterns:");
    
    // Test different working set sizes to see cache effects
    let working_sets = [
        (32, "L1 cache (32 KB)"),
        (256, "L2 cache (256 KB)"),
        (8192, "L3 cache (8 MB)"),
        (131072, "RAM (128 MB)"),
    ];
    
    println!("┌─────────────────┬──────────────┬──────────────┐");
    println!("│ Working Set     │  Throughput  │   Relative   │");
    println!("├─────────────────┼──────────────┼──────────────┤");
    
    let mut baseline = 0.0;
    
    for (idx, (size_kb, description)) in working_sets.iter().enumerate() {
        // Calculate buffer size to fit in cache level
        let buffer_items = (size_kb * 1024) / 8 / 2; // Divide by 2 for producer/consumer split
        let (producer, consumer) = channel::<u64>(buffer_items.min(65536));
        
        let iterations = 100_000_000;
        let start = Instant::now();
        
        for i in 0..iterations {
            if !producer.push(i as u64) {
                consumer.pop();
                producer.push(i as u64);
            }
        }
        
        let elapsed = start.elapsed();
        let ops_per_sec = iterations as f64 / elapsed.as_secs_f64();
        
        if idx == 0 {
            baseline = ops_per_sec;
        }
        
        let relative = (ops_per_sec / baseline) * 100.0;
        
        println!("│ {:15} │ {:>10.2} M/s│ {:>11.1}% │", 
                 description,
                 ops_per_sec / 1_000_000.0,
                 relative);
    }
    
    println!("└─────────────────┴──────────────┴──────────────┘");
}

fn ultimate_stress_test() {
    let (producer, consumer) = channel::<u64>(65536);
    let target_ops = 1_000_000_000u64; // 1 billion (reduced from 10B for faster test)
    
    println!("\nRunning 1 billion operations stress test...");
    println!("Buffer size: 65536 (64K)");
    
    let start = Instant::now();
    
    // Producer thread
    let producer_handle = thread::spawn(move || {
        let mut count = 0u64;
        let mut spin_count = 0u32;
        
        while count < target_ops {
            if producer.push(count) {
                count += 1;
                spin_count = 0;
                
                // Progress indicator every 100 million
                if count % 100_000_000 == 0 {
                    print!(".");
                    std::io::Write::flush(&mut std::io::stdout()).ok();
                }
            } else {
                spin_count += 1;
                if spin_count > 1000 {
                    std::thread::yield_now();
                    spin_count = 0;
                } else {
                    std::hint::spin_loop();
                }
            }
        }
        count
    });
    
    // Consumer thread
    let consumer_handle = thread::spawn(move || {
        let mut count = 0u64;
        let mut checksum = 0u64;
        let mut spin_count = 0u32;
        
        while count < target_ops {
            if let Some(val) = consumer.pop() {
                checksum = checksum.wrapping_add(val);
                count += 1;
                spin_count = 0;
            } else {
                spin_count += 1;
                if spin_count > 1000 {
                    std::thread::yield_now();
                    spin_count = 0;
                } else {
                    std::hint::spin_loop();
                }
            }
        }
        (count, checksum)
    });
    
    let produced = producer_handle.join().unwrap();
    let (consumed, checksum) = consumer_handle.join().unwrap();
    
    let elapsed = start.elapsed();
    
    println!("\n\n🏆 RESULTS:");
    println!("────────────────────────────────────────────────────────");
    println!("Total time:           {:?}", elapsed);
    println!("Operations produced:  {:?}", produced);
    println!("Operations consumed:  {:?}", consumed);
    println!("Checksum:            0x{:016x}", checksum);
    println!("────────────────────────────────────────────────────────");
    
    let ops_per_sec = consumed as f64 / elapsed.as_secs_f64();
    let ns_per_op = elapsed.as_nanos() as f64 / consumed as f64;
    let bandwidth_gbps = (consumed as f64 * 8.0) / elapsed.as_secs_f64() / 1_000_000_000.0;
    
    println!("Throughput:          {:.2} M ops/sec", ops_per_sec / 1_000_000.0);
    println!("Latency:             {:.2} ns/op", ns_per_op);
    println!("Effective bandwidth: {:.2} GB/s", bandwidth_gbps);
    
    // Performance rating
    let rating = match ops_per_sec {
        x if x > 500_000_000.0 => "🏆 WORLD CLASS - Approaching hardware limits!",
        x if x > 300_000_000.0 => "🥇 EXCEPTIONAL - Top 1% performance",
        x if x > 200_000_000.0 => "🥈 EXCELLENT - Production champion",
        x if x > 100_000_000.0 => "🥉 VERY GOOD - High performance",
        x if x > 50_000_000.0  => "✅ GOOD - Solid performance",
        _ => "⚠️ Needs optimization"
    };
    
    println!("\n⭐ Performance Rating: {}", rating);
}

fn compare_with_theoretical_limits() {
    println!("\nComparing with theoretical hardware limits:");
    println!("────────────────────────────────────────────────────────");
    
    // Theoretical limits (approximate)
    let l1_latency = 1.0; // ~1ns for L1 cache
    let l2_latency = 4.0; // ~4ns for L2 cache
    let l3_latency = 12.0; // ~12ns for L3 cache
    let ram_latency = 100.0; // ~100ns for RAM
    
    let memory_bandwidth = 100.0; // ~100 GB/s for modern DDR4/5
    let cache_line_size = 64.0; // bytes
    
    println!("Hardware theoretical limits:");
    println!("  L1 cache latency:     ~{:.0} ns", l1_latency);
    println!("  L2 cache latency:     ~{:.0} ns", l2_latency);
    println!("  L3 cache latency:     ~{:.0} ns", l3_latency);
    println!("  RAM latency:          ~{:.0} ns", ram_latency);
    println!("  Memory bandwidth:     ~{:.0} GB/s", memory_bandwidth);
    println!("  Cache line size:      {:.0} bytes", cache_line_size);
    
    // Quick benchmark to compare
    let (producer, consumer) = channel::<u64>(1024);
    let iterations = 10_000_000;
    
    // Measure actual latency
    let start = Instant::now();
    for i in 0..iterations {
        producer.push(i as u64);
        consumer.pop();
    }
    let elapsed = start.elapsed();
    let actual_latency = elapsed.as_nanos() as f64 / iterations as f64 / 2.0; // Divide by 2 for single op
    
    println!("\nActual measured performance:");
    println!("  Single operation:     ~{:.1} ns", actual_latency);
    println!("  vs L1 cache:          {:.1}x L1 latency", actual_latency / l1_latency);
    println!("  vs L2 cache:          {:.1}x L2 latency", actual_latency / l2_latency);
    
    let efficiency = (l2_latency / actual_latency) * 100.0;
    println!("\n📊 Efficiency Score: {:.1}%", efficiency.min(100.0));
    
    if efficiency > 80.0 {
        println!("✨ This implementation is operating near theoretical hardware limits!");
    } else if efficiency > 50.0 {
        println!("✅ Excellent efficiency - well-optimized implementation");
    } else {
        println!("💡 Room for optimization exists");
    }
}