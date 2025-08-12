use std::sync::atomic::{AtomicUsize, Ordering, fence};
use std::cell::Cell;
use std::mem::MaybeUninit;
use std::ptr;
use std::marker::PhantomData;
use std::alloc::{alloc, dealloc, Layout};

/// Pads a value to prevent false sharing between cache lines
/// Uses alignment rather than manual padding for simplicity
#[repr(C, align(64))]
struct CachePadded<T> {
    value: T,
}

impl<T> CachePadded<T> {
    fn new(value: T) -> Self {
        Self { value }
    }
}

/// The shared state between producer and consumer
/// 
/// This structure contains only the shared atomic indices.
/// Producer and consumer each have their own handles with cached values.
struct RingBufferCore<T> {
    /// Write position - updated only by producer
    write_pos: CachePadded<AtomicUsize>,
    
    /// Read position - updated only by consumer  
    read_pos: CachePadded<AtomicUsize>,
    
    /// Buffer capacity (must be power of 2)
    capacity: usize,
    
    /// Mask for fast modulo (capacity - 1)
    mask: usize,
    
    /// The actual buffer
    buffer: *mut MaybeUninit<T>,
    
    /// Marker for T's lifetime
    _marker: PhantomData<T>,
}

unsafe impl<T: Send> Send for RingBufferCore<T> {}
unsafe impl<T: Send> Sync for RingBufferCore<T> {}

impl<T> RingBufferCore<T> {
    /// Create a new ring buffer core with the specified capacity
    /// Capacity will be rounded up to the next power of 2
    fn new(capacity: usize) -> *mut Self {
        assert!(capacity > 0, "Capacity must be greater than 0");
        
        // Round up to power of 2 for efficient masking
        let capacity = capacity.next_power_of_two();
        let mask = capacity - 1;
        
        // Allocate aligned memory for the buffer
        let layout = Layout::array::<MaybeUninit<T>>(capacity)
            .expect("Failed to create layout");
        
        let buffer = unsafe { 
            let ptr = alloc(layout) as *mut MaybeUninit<T>;
            if ptr.is_null() {
                panic!("Failed to allocate memory for ring buffer");
            }
            ptr
        };
        
        // Allocate the core structure
        let core = Box::new(RingBufferCore {
            write_pos: CachePadded::new(AtomicUsize::new(0)),
            read_pos: CachePadded::new(AtomicUsize::new(0)),
            capacity,
            mask,
            buffer,
            _marker: PhantomData,
        });
        
        Box::into_raw(core)
    }
    
    /// Destroy the ring buffer core and free all memory
    unsafe fn destroy(core: *mut Self) {
        unsafe {
            let core = Box::from_raw(core);
            
            // Important: We do NOT try to drop items here
            // That's the responsibility of Producer/Consumer drop
            
            // Deallocate the buffer
            let layout = Layout::array::<MaybeUninit<T>>(core.capacity)
                .expect("Failed to create layout");
            dealloc(core.buffer as *mut u8, layout);
            
            // core is automatically dropped as it goes out of scope
        }
    }
}

/// Producer handle for the ring buffer
/// 
/// Only one producer should exist at a time.
/// This handle owns the write position and cached read position.
pub struct Producer<T> {
    /// Pointer to shared core
    core: *mut RingBufferCore<T>,
    
    /// Cached read position (local to producer)
    /// This avoids constantly reading the atomic read_pos
    cached_read: Cell<usize>,
    
    /// Cached write position (local to producer)
    /// This is our authoritative write position
    cached_write: Cell<usize>,
    
    /// Ensure T is Send
    _marker: PhantomData<T>,
}

unsafe impl<T: Send> Send for Producer<T> {}

impl<T> Producer<T> {
    /// Push an item to the buffer
    /// Returns false if buffer is full
    #[inline]
    pub fn push(&self, value: T) -> bool {
        unsafe {
            let core = &*self.core;
            let write = self.cached_write.get();
            let next_write = write + 1;
            
            // Check if buffer is full using cached read position
            // Note: We can only use capacity-1 slots to distinguish full from empty
            if next_write - self.cached_read.get() > core.capacity - 1 {
                // Update cached read position with fresh value
                let fresh_read = core.read_pos.value.load(Ordering::Acquire);
                self.cached_read.set(fresh_read);
                
                // Check again with fresh read position
                if next_write - fresh_read > core.capacity - 1 {
                    return false;
                }
            }
            
            // Write the value to the buffer
            let slot = core.buffer.add(write & core.mask);
            ptr::write(slot, MaybeUninit::new(value));
            
            // CRITICAL: Ensure the write completes before publishing
            // The Release ordering ensures all previous writes (including ptr::write)
            // complete before this store becomes visible
            fence(Ordering::Release);
            
            // Update our cached position
            self.cached_write.set(next_write);
            
            // Publish the new write position for the consumer
            core.write_pos.value.store(next_write, Ordering::Release);
            
            true
        }
    }
    
    /// Try to push multiple items at once (batch operation)
    /// Returns the number of items successfully pushed
    #[inline]
    pub fn push_slice(&self, values: &[T]) -> usize 
    where 
        T: Copy
    {
        if values.is_empty() {
            return 0;
        }
        
        unsafe {
            let core = &*self.core;
            let write = self.cached_write.get();
            
            // Get fresh read position for accurate capacity check
            let read = core.read_pos.value.load(Ordering::Acquire);
            self.cached_read.set(read);
            
            // Calculate available space (remember: capacity-1 usable slots)
            let available = (core.capacity - 1) - (write - read);
            let to_write = values.len().min(available);
            
            if to_write == 0 {
                return 0;
            }
            
            // Write items one by one to handle MaybeUninit properly
            for i in 0..to_write {
                let slot_idx = (write + i) & core.mask;
                let slot = core.buffer.add(slot_idx);
                ptr::write(slot, MaybeUninit::new(values[i]));
            }
            
            // Ensure all writes complete before publishing
            fence(Ordering::Release);
            
            let new_write = write + to_write;
            self.cached_write.set(new_write);
            
            // Publish the new write position
            core.write_pos.value.store(new_write, Ordering::Release);
            
            to_write
        }
    }
    
    /// Get the number of items that can be pushed without blocking
    #[inline]
    pub fn available_capacity(&self) -> usize {
        unsafe {
            let core = &*self.core;
            let write = self.cached_write.get();
            let read = core.read_pos.value.load(Ordering::Acquire);
            self.cached_read.set(read);
            
            (core.capacity - 1).saturating_sub(write - read)
        }
    }
}

impl<T> Drop for Producer<T> {
    fn drop(&mut self) {
        // Producer is being dropped - no special cleanup needed
        // The actual cleanup happens when both Producer and Consumer are dropped
    }
}

/// Consumer handle for the ring buffer
/// 
/// Only one consumer should exist at a time.
/// This handle owns the read position and cached write position.
pub struct Consumer<T> {
    /// Pointer to shared core
    core: *mut RingBufferCore<T>,
    
    /// Cached write position (local to consumer)
    /// This avoids constantly reading the atomic write_pos
    cached_write: Cell<usize>,
    
    /// Cached read position (local to consumer)
    /// This is our authoritative read position
    cached_read: Cell<usize>,
    
    /// Ensure T is Send
    _marker: PhantomData<T>,
}

unsafe impl<T: Send> Send for Consumer<T> {}

impl<T> Consumer<T> {
    /// Pop an item from the buffer
    /// Returns None if buffer is empty
    #[inline]
    pub fn pop(&self) -> Option<T> {
        unsafe {
            let core = &*self.core;
            let read = self.cached_read.get();
            
            // Check if buffer is empty using cached write position
            if read == self.cached_write.get() {
                // Update cached write position with fresh value
                let fresh_write = core.write_pos.value.load(Ordering::Acquire);
                self.cached_write.set(fresh_write);
                
                // Check again with fresh write position
                if read == fresh_write {
                    return None;
                }
            }
            
            // Read the value from the buffer
            let slot = core.buffer.add(read & core.mask);
            let value = ptr::read(slot).assume_init();
            
            // Update our cached position
            self.cached_read.set(read + 1);
            
            // Publish the new read position for the producer
            // Release ordering ensures the read completes before slot is reused
            core.read_pos.value.store(read + 1, Ordering::Release);
            
            Some(value)
        }
    }
    
    /// Try to pop multiple items at once (batch operation)
    /// Returns the number of items successfully popped
    #[inline]
    pub fn pop_slice(&self, out: &mut [T]) -> usize
    where
        T: Copy
    {
        if out.is_empty() {
            return 0;
        }
        
        unsafe {
            let core = &*self.core;
            let read = self.cached_read.get();
            
            // Get fresh write position for accurate item count
            let write = core.write_pos.value.load(Ordering::Acquire);
            self.cached_write.set(write);
            
            // Calculate available items
            let available = write - read;
            let to_read = out.len().min(available);
            
            if to_read == 0 {
                return 0;
            }
            
            // Read items one by one to handle MaybeUninit properly
            for i in 0..to_read {
                let slot_idx = (read + i) & core.mask;
                let slot = core.buffer.add(slot_idx);
                out[i] = ptr::read(slot).assume_init();
            }
            
            let new_read = read + to_read;
            self.cached_read.set(new_read);
            
            // Publish the new read position
            core.read_pos.value.store(new_read, Ordering::Release);
            
            to_read
        }
    }
    
    /// Get the number of items available to pop
    #[inline]
    pub fn available(&self) -> usize {
        unsafe {
            let core = &*self.core;
            let read = self.cached_read.get();
            let write = core.write_pos.value.load(Ordering::Acquire);
            self.cached_write.set(write);
            
            write - read
        }
    }
    
    /// Peek at the next item without removing it
    #[inline]
    pub fn peek(&self) -> Option<&T> {
        unsafe {
            let core = &*self.core;
            let read = self.cached_read.get();
            
            // Check if buffer is empty
            let write = core.write_pos.value.load(Ordering::Acquire);
            self.cached_write.set(write);
            
            if read == write {
                return None;
            }
            
            // Return a reference to the next item
            let slot = core.buffer.add(read & core.mask);
            Some(&*(*slot).as_ptr())
        }
    }
}

impl<T> Drop for Consumer<T> {
    fn drop(&mut self) {
        // Note: We don't drop items here because:
        // 1. When using RingBuffer::new(), the RingBuffer itself manages cleanup
        // 2. When using channel(), the core is leaked and cleanup isn't needed
        // This prevents double-free issues and null pointer dereferences
    }
}

/// Ring buffer handle that manages the shared core
/// 
/// This struct is responsible for cleaning up the core when
/// both producer and consumer are dropped.
pub struct RingBuffer<T> {
    core: *mut RingBufferCore<T>,
}

unsafe impl<T: Send> Send for RingBuffer<T> {}
unsafe impl<T: Send> Sync for RingBuffer<T> {}

impl<T> RingBuffer<T> {
    /// Create a new ring buffer with the specified capacity
    /// 
    /// Capacity will be rounded up to the next power of 2.
    /// Returns the ring buffer and separated producer/consumer handles.
    pub fn new(capacity: usize) -> (Producer<T>, Consumer<T>, Self) {
        let core = RingBufferCore::new(capacity);
        
        let producer = Producer {
            core,
            cached_read: Cell::new(0),
            cached_write: Cell::new(0),
            _marker: PhantomData,
        };
        
        let consumer = Consumer {
            core,
            cached_write: Cell::new(0),
            cached_read: Cell::new(0),
            _marker: PhantomData,
        };
        
        let rb = RingBuffer { core };
        
        (producer, consumer, rb)
    }
    
    /// Get the capacity of the ring buffer
    pub fn capacity(&self) -> usize {
        unsafe { (*self.core).capacity }
    }
}

impl<T> Drop for RingBuffer<T> {
    fn drop(&mut self) {
        unsafe {
            let core = &*self.core;
            
            // Drop any remaining items in the buffer
            let read = core.read_pos.value.load(Ordering::Acquire);
            let write = core.write_pos.value.load(Ordering::Acquire);
            let mut current = read;
            
            while current != write {
                let slot = core.buffer.add(current & core.mask);
                ptr::drop_in_place((*slot).as_mut_ptr());
                current += 1;
            }
            
            // Now destroy the core
            RingBufferCore::destroy(self.core);
        }
    }
}

/// Convenience function to create a new SPSC channel with a ring buffer
/// 
/// This simplified version creates producer and consumer that share ownership
/// of the core. The core will be leaked but this is safe for long-lived channels.
/// For proper cleanup, use RingBuffer::new() instead.
pub fn channel<T>(capacity: usize) -> (Producer<T>, Consumer<T>) {
    let core = RingBufferCore::new(capacity);
    
    let producer = Producer {
        core,
        cached_read: Cell::new(0),
        cached_write: Cell::new(0),
        _marker: PhantomData,
    };
    
    let consumer = Consumer {
        core,
        cached_write: Cell::new(0),
        cached_read: Cell::new(0),
        _marker: PhantomData,
    };
    
    (producer, consumer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    
    #[test]
    fn test_basic_push_pop() {
        let (producer, consumer, _rb) = RingBuffer::new(4);
        
        // Test push and pop
        assert!(producer.push(1));
        assert!(producer.push(2));
        assert!(producer.push(3));
        
        assert_eq!(consumer.pop(), Some(1));
        assert_eq!(consumer.pop(), Some(2));
        assert_eq!(consumer.pop(), Some(3));
        assert_eq!(consumer.pop(), None);
    }
    
    #[test]
    fn test_buffer_full() {
        let (producer, consumer, _rb) = RingBuffer::new(4); // Actual capacity is 4
        
        // Fill buffer (can only use capacity-1 slots)
        assert!(producer.push(1));
        assert!(producer.push(2));
        assert!(producer.push(3));
        assert!(!producer.push(4)); // Should fail - buffer full
        
        // Pop one and try again
        assert_eq!(consumer.pop(), Some(1));
        assert!(producer.push(4)); // Should succeed now
        assert!(!producer.push(5)); // Should fail again
    }
    
    #[test]
    fn test_power_of_two_rounding() {
        let (producer, consumer, rb) = RingBuffer::new(5); // Should round up to 8
        assert_eq!(rb.capacity(), 8);
        
        // Should be able to push 7 items (capacity-1)
        for i in 1..=7 {
            assert!(producer.push(i), "Failed to push {}", i);
        }
        assert!(!producer.push(8)); // Should fail
        
        // Verify all items
        for i in 1..=7 {
            assert_eq!(consumer.pop(), Some(i));
        }
    }
    
    #[test]
    fn test_batch_operations() {
        let (producer, consumer, _rb) = RingBuffer::new(16);
        
        // Test push_slice
        let data = [1, 2, 3, 4, 5];
        assert_eq!(producer.push_slice(&data), 5);
        
        // Test pop_slice
        let mut out = [0; 3];
        assert_eq!(consumer.pop_slice(&mut out), 3);
        assert_eq!(out, [1, 2, 3]);
        
        // Pop remaining
        let mut out = [0; 5];
        assert_eq!(consumer.pop_slice(&mut out), 2);
        assert_eq!(&out[..2], &[4, 5]);
    }
    
    #[test]
    fn test_available_capacity() {
        let (producer, consumer, _rb) = RingBuffer::new(8);
        
        assert_eq!(producer.available_capacity(), 7); // capacity-1
        
        producer.push(1);
        producer.push(2);
        assert_eq!(producer.available_capacity(), 5);
        
        consumer.pop();
        assert_eq!(producer.available_capacity(), 6);
    }
    
    #[test]
    fn test_available_items() {
        let (producer, consumer, _rb) = RingBuffer::new(8);
        
        assert_eq!(consumer.available(), 0);
        
        producer.push(1);
        producer.push(2);
        producer.push(3);
        assert_eq!(consumer.available(), 3);
        
        consumer.pop();
        assert_eq!(consumer.available(), 2);
    }
    
    #[test]
    fn test_peek() {
        let (producer, consumer, _rb) = RingBuffer::new(4);
        
        assert_eq!(consumer.peek(), None);
        
        producer.push(42);
        producer.push(43);
        
        // Peek shouldn't consume
        assert_eq!(consumer.peek(), Some(&42));
        assert_eq!(consumer.peek(), Some(&42));
        
        // Pop and peek next
        assert_eq!(consumer.pop(), Some(42));
        assert_eq!(consumer.peek(), Some(&43));
    }
    
    #[test]
    fn test_concurrent_single_value() {
        let (producer, consumer, _rb) = RingBuffer::new(4);
        
        let handle = thread::spawn(move || {
            for i in 0..1000 {
                while !producer.push(i) {
                    thread::yield_now();
                }
            }
        });
        
        let mut received = Vec::new();
        while received.len() < 1000 {
            if let Some(val) = consumer.pop() {
                received.push(val);
            } else {
                thread::yield_now();
            }
        }
        
        handle.join().unwrap();
        
        // Verify all values received in order
        for (i, &val) in received.iter().enumerate() {
            assert_eq!(val, i);
        }
    }
    
    #[test]
    fn test_concurrent_batch() {
        let (producer, consumer, _rb) = RingBuffer::new(64);
        let total = 100_000;
        
        let handle = thread::spawn(move || {
            let mut sent = 0;
            while sent < total {
                let batch: Vec<usize> = (sent..(sent + 10).min(total)).collect();
                let pushed = producer.push_slice(&batch);
                sent += pushed;
                if pushed == 0 {
                    thread::yield_now();
                }
            }
        });
        
        let mut received = Vec::new();
        let mut buffer = [0; 16];
        while received.len() < total {
            let popped = consumer.pop_slice(&mut buffer);
            received.extend_from_slice(&buffer[..popped]);
            if popped == 0 {
                thread::yield_now();
            }
        }
        
        handle.join().unwrap();
        
        // Verify all values received in order
        for (i, &val) in received.iter().enumerate() {
            assert_eq!(val, i);
        }
    }
    
    #[test]
    fn test_drop_cleanup() {
        // Test that items are properly dropped
        static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);
        
        struct DropCounter(#[allow(dead_code)] usize);
        impl Drop for DropCounter {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Ordering::SeqCst);
            }
        }
        
        DROP_COUNT.store(0, Ordering::SeqCst);
        
        {
            let (producer, consumer, _rb) = RingBuffer::new(8);
            
            producer.push(DropCounter(1));
            producer.push(DropCounter(2));
            producer.push(DropCounter(3));
            
            // Pop one item
            let _ = consumer.pop();
            assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
            
            // Remaining items should be dropped when consumer is dropped
        }
        
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 3);
    }
    
    #[test]
    fn test_channel_convenience() {
        // Test the convenience channel function
        let (producer, consumer) = channel::<i32>(8);
        
        producer.push(1);
        producer.push(2);
        
        assert_eq!(consumer.pop(), Some(1));
        assert_eq!(consumer.pop(), Some(2));
        assert_eq!(consumer.pop(), None);
    }
    
    #[test]
    fn test_wrap_around() {
        let (producer, consumer, _rb) = RingBuffer::new(4);
        
        // Fill and drain multiple times to test wrap-around
        for cycle in 0..10 {
            for i in 0..3 {
                assert!(producer.push(cycle * 10 + i));
            }
            for i in 0..3 {
                assert_eq!(consumer.pop(), Some(cycle * 10 + i));
            }
        }
    }
    
    #[test]
    fn test_stress_concurrent() {
        let (producer, consumer, _rb) = RingBuffer::new(1024);
        let iterations = 1_000_000;
        let done = Arc::new(AtomicBool::new(false));
        let done_clone = done.clone();
        
        let producer_handle = thread::spawn(move || {
            for i in 0..iterations {
                while !producer.push(i) {
                    thread::yield_now();
                }
            }
            done_clone.store(true, Ordering::Release);
        });
        
        let consumer_handle = thread::spawn(move || {
            let mut count = 0;
            let mut last = None;
            
            while !done.load(Ordering::Acquire) || consumer.available() > 0 {
                if let Some(val) = consumer.pop() {
                    // Verify ordering
                    if let Some(last_val) = last {
                        assert_eq!(val, last_val + 1, "Out of order at {}", count);
                    }
                    last = Some(val);
                    count += 1;
                } else {
                    thread::yield_now();
                }
            }
            
            assert_eq!(count, iterations, "Missing items");
            count
        });
        
        producer_handle.join().unwrap();
        let consumed = consumer_handle.join().unwrap();
        assert_eq!(consumed, iterations);
    }
    
    #[test]
    fn test_zero_sized_types() {
        #[derive(Debug, PartialEq)]
        struct ZeroSized;
        
        let (producer, consumer, _rb) = RingBuffer::new(4);
        
        producer.push(ZeroSized);
        producer.push(ZeroSized);
        
        assert_eq!(consumer.pop(), Some(ZeroSized));
        assert_eq!(consumer.pop(), Some(ZeroSized));
        assert_eq!(consumer.pop(), None);
    }
}
