// code1
pub mod unsafe_cache_line;

use std::sync::atomic::{AtomicUsize, Ordering, fence};
use std::cell::Cell;
use std::mem::MaybeUninit;
use std::ptr;
use std::marker::PhantomData;
use std::alloc::{alloc, dealloc, Layout};
use std::sync::Arc;

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
    fn new(capacity: usize) -> Arc<Self> {
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
        
        Arc::new(RingBufferCore {
            write_pos: CachePadded::new(AtomicUsize::new(0)),
            read_pos: CachePadded::new(AtomicUsize::new(0)),
            capacity,
            mask,
            buffer,
            _marker: PhantomData,
        })
    }
}

impl<T> Drop for RingBufferCore<T> {
    fn drop(&mut self) {
        unsafe {
            // First, drop any remaining items in the buffer
            let read = self.read_pos.value.load(Ordering::Acquire);
            let write = self.write_pos.value.load(Ordering::Acquire);
            
            let mut current = read;
            while current != write {
                let slot = self.buffer.add(current & self.mask);
                ptr::drop_in_place((*slot).as_mut_ptr());
                current += 1;
            }
            
            // Then deallocate the buffer
            let layout = Layout::array::<MaybeUninit<T>>(self.capacity)
                .expect("Failed to create layout");
            dealloc(self.buffer as *mut u8, layout);
        }
    }
}

/// Producer handle for the ring buffer
/// 
/// Only one producer should exist at a time.
/// This handle owns the write position and cached read position.
pub struct Producer<T> {
    /// Shared core (reference counted)
    core: Arc<RingBufferCore<T>>,
    
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
        let core = &self.core;
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
        
        unsafe {
            // Write the value to the buffer
            let slot = core.buffer.add(write & core.mask);
            ptr::write(slot, MaybeUninit::new(value));
        }
        
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
        
        let core = &self.core;
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
        
        unsafe {
            // Write items one by one to handle MaybeUninit properly
            for i in 0..to_write {
                let slot_idx = (write + i) & core.mask;
                let slot = core.buffer.add(slot_idx);
                ptr::write(slot, MaybeUninit::new(values[i]));
            }
        }
        
        // Ensure all writes complete before publishing
        fence(Ordering::Release);
        
        let new_write = write + to_write;
        self.cached_write.set(new_write);
        
        // Publish the new write position
        core.write_pos.value.store(new_write, Ordering::Release);
        
        to_write
    }
    
    /// Get the number of items that can be pushed without blocking
    #[inline]
    pub fn available_capacity(&self) -> usize {
        let core = &self.core;
        let write = self.cached_write.get();
        let read = core.read_pos.value.load(Ordering::Acquire);
        self.cached_read.set(read);
        
        (core.capacity - 1).saturating_sub(write - read)
    }
}

/// Consumer handle for the ring buffer
/// 
/// Only one consumer should exist at a time.
/// This handle owns the read position and cached write position.
pub struct Consumer<T> {
    /// Shared core (reference counted)
    core: Arc<RingBufferCore<T>>,
    
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
        let core = &self.core;
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
        
        unsafe {
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
        
        let core = &self.core;
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
        
        unsafe {
            // Read items one by one to handle MaybeUninit properly
            for i in 0..to_read {
                let slot_idx = (read + i) & core.mask;
                let slot = core.buffer.add(slot_idx);
                out[i] = ptr::read(slot).assume_init();
            }
        }
        
        let new_read = read + to_read;
        self.cached_read.set(new_read);
        
        // Publish the new read position
        core.read_pos.value.store(new_read, Ordering::Release);
        
        to_read
    }
    
    /// Get the number of items available to pop
    #[inline]
    pub fn available(&self) -> usize {
        let core = &self.core;
        let read = self.cached_read.get();
        let write = core.write_pos.value.load(Ordering::Acquire);
        self.cached_write.set(write);
        
        write - read
    }
    
    /// Peek at the next item without removing it
    #[inline]
    pub fn peek(&self) -> Option<&T> {
        let core = &self.core;
        let read = self.cached_read.get();
        
        // Check if buffer is empty
        let write = core.write_pos.value.load(Ordering::Acquire);
        self.cached_write.set(write);
        
        if read == write {
            return None;
        }
        
        unsafe {
            // Return a reference to the next item
            let slot = core.buffer.add(read & core.mask);
            Some(&*(*slot).as_ptr())
        }
    }
}

/// Create a new SPSC ring buffer with the specified capacity
/// 
/// Capacity will be rounded up to the next power of 2.
/// Returns producer and consumer handles.
pub fn channel<T>(capacity: usize) -> (Producer<T>, Consumer<T>) {
    let core = RingBufferCore::new(capacity);
    
    let producer = Producer {
        core: core.clone(),
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
    
    #[test]
    fn test_basic_operations() {
        let (producer, consumer) = channel(4);
        
        assert!(producer.push(1));
        assert!(producer.push(2));
        assert!(producer.push(3));
        assert!(!producer.push(4)); // Buffer full (can only use 3 of 4 slots)
        
        assert_eq!(consumer.pop(), Some(1));
        assert_eq!(consumer.pop(), Some(2));
        assert!(producer.push(4));
        assert!(producer.push(5));
        
        assert_eq!(consumer.pop(), Some(3));
        assert_eq!(consumer.pop(), Some(4));
        assert_eq!(consumer.pop(), Some(5));
        assert_eq!(consumer.pop(), None); // Buffer empty
    }
    
    #[test]
    fn test_spsc_channel() {
        let (producer, consumer) = channel(1024);
        let iterations = 1_000_000;
        
        let producer_thread = thread::spawn(move || {
            for i in 0..iterations {
                while !producer.push(i) {
                    std::hint::spin_loop();
                }
            }
        });
        
        let consumer_thread = thread::spawn(move || {
            let mut sum = 0i64;
            for _ in 0..iterations {
                loop {
                    if let Some(val) = consumer.pop() {
                        sum += val as i64;
                        break;
                    }
                    std::hint::spin_loop();
                }
            }
            sum
        });
        
        producer_thread.join().unwrap();
        let sum = consumer_thread.join().unwrap();
        
        let expected = (iterations as i64 * (iterations as i64 - 1)) / 2;
        assert_eq!(sum, expected);
    }
    
    #[test]
    fn test_batch_operations() {
        let (producer, consumer) = channel(8);
        
        let data = [1, 2, 3, 4, 5];
        assert_eq!(producer.push_slice(&data), 5);
        
        let mut out = [0; 3];
        assert_eq!(consumer.pop_slice(&mut out), 3);
        assert_eq!(out, [1, 2, 3]);
        
        let more_data = [6, 7, 8, 9];
        // After popping 3, we have 2 items in buffer (4, 5)
        // With capacity 8 (7 usable), we can fit 5 more items
        // So all 4 items should be pushed successfully
        assert_eq!(producer.push_slice(&more_data), 4);
        
        let mut out2 = [0; 10];
        assert_eq!(consumer.pop_slice(&mut out2), 6); // Now we have 6 items total
        assert_eq!(&out2[..6], &[4, 5, 6, 7, 8, 9]);
    }
    
    #[test]
    fn test_drop_cleanup() {
        let (producer, consumer) = channel(8);
        
        // Push some values that need dropping
        producer.push(String::from("hello"));
        producer.push(String::from("world"));
        
        // Drop consumer - should clean up remaining items
        drop(consumer);
        drop(producer);
    }
    
    #[test]
    fn test_peek() {
        let (producer, consumer) = channel(4);
        
        assert!(consumer.peek().is_none());
        
        producer.push(42);
        assert_eq!(consumer.peek(), Some(&42));
        assert_eq!(consumer.peek(), Some(&42)); // Peek doesn't consume
        
        assert_eq!(consumer.pop(), Some(42));
        assert!(consumer.peek().is_none());
    }
    
    #[test]
    fn test_available_counts() {
        let (producer, consumer) = channel(4);
        
        assert_eq!(producer.available_capacity(), 3); // Can use capacity-1
        assert_eq!(consumer.available(), 0);
        
        producer.push(1);
        producer.push(2);
        
        assert_eq!(producer.available_capacity(), 1);
        assert_eq!(consumer.available(), 2);
        
        consumer.pop();
        
        assert_eq!(producer.available_capacity(), 2);
        assert_eq!(consumer.available(), 1);
    }
}