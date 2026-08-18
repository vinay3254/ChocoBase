# Phase 1 Tasks

- [x] Add `storage::buffer_pool` with frame metadata, per-frame `RwLock`, pin/unpin guards, and bounded eviction.
- [x] Add `storage::lock_manager` with shared/exclusive strict-2PL lock acquisition and transaction lock tokens.
- [x] Add `SharedDatabase` in the engine as an `Arc`-backed concurrent facade while preserving `Database`.
- [x] Route shared-handle reads through shared locks and writes through exclusive locks; preserve explicit transaction behavior.
- [x] Add unit tests for buffer-pool and lock-manager behavior.
- [x] Add real multi-threaded database stress tests covering concurrent readers and serialized writers.
- [x] Run the full Rust test suite, including process-kill crash recovery, and record results.
- [x] Document deferred MVCC and WAL/checkpoint work for a later phase.
