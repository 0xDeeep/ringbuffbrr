This Rust code implements a single-producer, single-consumer (SPSC) lock-free ring buffer (queue) with efficient atomic synchronization and cache line padding to avoid false sharing. It’s a highly performant concurrent data structure for passing data between two threads without blocking or locking.

Important considerations:
This queue supports exactly one producer and one consumer. Using multiple producers or consumers simultaneously is not safe with this code as is.
The capacity must be a power of two (or it is rounded up to one), ensuring efficient modulo via masking.
This implementation uses unsafe code and manual memory management — make sure to thoroughly test and audit before using it in critical production.

> cargo run --release --example verify_perf

> cargo run --release --example comparison
