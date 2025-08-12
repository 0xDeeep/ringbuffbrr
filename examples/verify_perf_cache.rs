use ringbuffbrr::unsafe_cache_line::{channel, RingBuffer};
use std::time::Instant;
use std::thread;

fn main() {
    println!("=================================================");
    println!("  Cache-Line Optimized Ring Buffer Performance");
    println!("=================================================\n");

    // Test different buffer sizes
    let sizes = [256, 1024, 4096, 16384, 65536];
    
    println!("📊 Single-threaded Performance Tests");
    println!("─────────────────────────────────────");
    for size in sizes {
        println!("Buffer size: {}", size);
        test_single_thread_performance(size);
        println!();
    }
    
    println!("\n📊 Multi-threaded Performance Tests");
    println!("─────────────────────────────────────");
    for size in sizes {
        println!("Buffer size: {}", size);
        test_concurrent_performance(size);
        println!();
    }
    
    // Big test - 1 billion operations
    println!("=================================================");
    println!("   ULTIMATE TEST: 1 Billion Operations (Cache-Line)");
    println!("=================================================\n");
    
    ultimate_test();
    
    // Batch operations test
    println!("\n=================================================");
    println!("        Batch Operations Performance");
    println!("=================================================\n");
    
    test_batch_performance();
}

fn test_single_thread_performance(size: usize) {
    let (producer, consumer) = channel::<u64>(size);
    let iterations = 100_000_000; // 100 million
    
    // Warmup
    for i in 0..10_000 {
        producer.push(i);
        consumer.pop();
    }
    
    let start = Instant::now();
    
    for i in 0..iterations {
        if !producer.push(i) {
            consumer.pop();
            producer.push(i);
        }
    }
    
    // Drain remaining
    while consumer.pop().is_some() {}
    
    let elapsed = start.elapsed();
    let ops_per_sec = iterations as f64 / elapsed.as_secs_f64();
    let ns_per_op = elapsed.as_nanos() as f64 / iterations as f64;
    
    println!("  → {:>6.0}M ops/sec, {:.2} ns/op", 
             ops_per_sec / 1_000_000.0, ns_per_op);
}

fn test_concurrent_performance(size: usize) {
    let (producer, consumer, _rb) = RingBuffer::new(size);
    let iterations = 100_000_000u64; // 100 million
    
    let start = Instant::now();
    
    let producer_handle = thread::spawn(move || {
        for i in 0..iterations {
            while !producer.push(i) {
                std::hint::spin_loop();
            }
        }
    });
    
    let consumer_handle = thread::spawn(move || {
        let mut count = 0u64;
        while count < iterations {
            if consumer.pop().is_some() {
                count += 1;
            } else {
                std::hint::spin_loop();
            }
        }
        count
    });
    
    producer_handle.join().unwrap();
    let consumed = consumer_handle.join().unwrap();
    
    let elapsed = start.elapsed();
    let ops_per_sec = iterations as f64 / elapsed.as_secs_f64();
    let ns_per_op = elapsed.as_nanos() as f64 / iterations as f64;
    
    println!("  → {:>6.0}M ops/sec, {:.2} ns/op (consumed: {})", 
             ops_per_sec / 1_000_000.0, ns_per_op, consumed);
}

fn ultimate_test() {
    let (producer, consumer) = channel::<u64>(4096);
    let iterations = 1_000_000_000; // 1 billion
    
    // Warmup with cache-line aware operations
    println!("Warming up cache lines...");
    for i in 0..1_000_000 {
        if !producer.push(i) {
            consumer.pop();
            producer.push(i);
        }
    }
    
    // Clear the buffer
    while consumer.pop().is_some() {}
    
    println!("Starting 1 billion operations...");
    let start = Instant::now();
    
    for i in 0..iterations {
        if !producer.push(i) {
            consumer.pop();
            producer.push(i);
        }
    }
    
    let elapsed = start.elapsed();
    
    println!("\n📊 CACHE-LINE OPTIMIZED RESULTS:");
    println!("────────────────────────────────────────");
    println!("Total time:          {:?}", elapsed);
    println!("Total operations:    {:?}", iterations);
    println!("Ops/sec:            {:.0}M", iterations as f64 / elapsed.as_secs_f64() / 1_000_000.0);
    println!("Nanoseconds per op: {:.2} ns", elapsed.as_nanos() as f64 / iterations as f64);
    println!("────────────────────────────────────────");
    
    // Calculate theoretical memory bandwidth
    let bytes_per_op = 8; // u64 = 8 bytes
    let total_bytes = iterations * bytes_per_op;
    let bandwidth_gbps = (total_bytes as f64 / elapsed.as_secs_f64()) / 1_000_000_000.0;
    println!("Effective bandwidth: {:.2} GB/s", bandwidth_gbps);
    
    // Cache efficiency estimate
    let cache_line_size = 64; // bytes
    let ops_per_cache_line = cache_line_size / bytes_per_op;
    let cache_lines_touched = iterations / ops_per_cache_line;
    println!("Cache lines touched: ~{:.0}M", cache_lines_touched as f64 / 1_000_000.0);
    
    // Performance rating
    let ops_per_sec = iterations as f64 / elapsed.as_secs_f64();
    let rating = match ops_per_sec {
        x if x > 800_000_000.0 => "🏆 LEGENDARY - Cache-line optimization achieved!",
        x if x > 500_000_000.0 => "🥇 WORLD CLASS - Near theoretical limit!",
        x if x > 200_000_000.0 => "🥈 EXCELLENT - Top tier performance",
        x if x > 100_000_000.0 => "🥉 VERY GOOD - Production ready",
        x if x > 50_000_000.0  => "Good - Solid performance",
        _ => "Needs optimization"
    };
    
    println!("\nPerformance Rating: {}", rating);
}

fn test_batch_performance() {
    let (producer, consumer, _rb) = RingBuffer::<u64>::new(4096);
    let batch_sizes = [1usize, 4, 8, 16, 32, 64, 128, 256];
    let total_items = 100_000_000u64;
    
    println!("Testing batch operations with different sizes:");
    println!("───────────────────────────────────────────────");
    
    for &batch_size in &batch_sizes {
        // Prepare batch
        let batch: Vec<u64> = (0..batch_size as u64).collect();
        let mut receive_buffer = vec![0u64; batch_size];
        
        // Warmup
        for _ in 0..1000 {
            producer.push_slice(&batch);
            consumer.pop_slice(&mut receive_buffer);
        }
        
        let start = Instant::now();
        let iterations = total_items / batch_size as u64;
        
        for i in 0..iterations {
            // Update batch values
            let batch: Vec<u64> = ((i * batch_size as u64)..((i + 1) * batch_size as u64)).collect();
            
            while producer.push_slice(&batch) != batch_size {
                consumer.pop_slice(&mut receive_buffer);
            }
        }
        
        // Drain remaining
        while consumer.pop_slice(&mut receive_buffer) > 0 {}
        
        let elapsed = start.elapsed();
        let items_per_sec = total_items as f64 / elapsed.as_secs_f64();
        let ns_per_item = elapsed.as_nanos() as f64 / total_items as f64;
        
        println!("Batch size {:3}: {:>6.0}M items/sec, {:.2} ns/item", 
                 batch_size, items_per_sec / 1_000_000.0, ns_per_item);
    }
}