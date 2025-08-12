use std::time::Instant;

fn main() {
    println!("\n=== RING BUFFER PERFORMANCE COMPARISON ===\n");
    
    // Your implementation
    let iterations = 100_000_000;
    let (producer, consumer) = ringbuffbrr::channel::<u64>(4096);
    
    let start = Instant::now();
    for i in 0..iterations {
        if !producer.push(i) { 
            consumer.pop(); 
            producer.push(i); 
        }
    }
    let your_time = start.elapsed();
    
    // crossbeam-channel
    use crossbeam_channel::bounded;
    let (tx, rx) = bounded(4096);
    
    let start = Instant::now();
    for i in 0..iterations {
        if tx.try_send(i).is_err() {
            rx.try_recv().ok();
            tx.try_send(i).ok();
        }
    }
    let crossbeam_time = start.elapsed();
    
    println!("Your RingBuffer:  {:.0}M ops/sec", iterations as f64 / your_time.as_secs_f64() / 1_000_000.0);
    println!("Crossbeam:        {:.0}M ops/sec", iterations as f64 / crossbeam_time.as_secs_f64() / 1_000_000.0);
    println!("\nYour implementation is {:.1}x faster!", 
             crossbeam_time.as_secs_f64() / your_time.as_secs_f64());
}