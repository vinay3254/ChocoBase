# Phase 1: Concurrency Foundation Requirements

## Goal

Make ChocoBase safe for concurrent in-process use while preserving the existing embedded API and all current durability behavior.

## Functional requirements

1. The storage layer must expose a thread-safe buffer-pool abstraction.
   - Frames are identified by page number.
   - Each frame tracks pin count and dirty state.
   - Each frame has a reader/writer latch so concurrent readers can share a page and writers are exclusive.
   - Eviction must never remove a pinned frame.
2. A lock manager must implement strict two-phase locking for database operations.
   - Shared locks may coexist.
   - An exclusive lock excludes all other shared or exclusive locks.
   - Locks are held until transaction end and released on commit or rollback.
   - Conflicting requests block rather than allowing dirty reads or lost writes.
3. The existing `Database` API and SQL behavior must continue to work.
   - Existing transaction, persistence, journal, and crash-recovery tests remain green.
   - Existing single-threaded callers do not need to change.
4. A shareable database handle must support concurrent readers and serialized writers from multiple threads in one process.
5. Transaction failure must release latches, pins, and logical locks so later operations cannot deadlock.
6. The journal must remain crash-safe under concurrent-handle use. A process killed during a transaction must recover to the pre-transaction state.

## Non-functional requirements

- No unsafe Rust.
- No on-disk format change in Phase 1.
- Public synchronization primitives must be documented.
- Stress tests must use real threads; at least one test must demonstrate writer serialization and reader progress.

## Explicit scope decisions

- Full MVCC and snapshot isolation are deferred. Phase 1 provides strict 2PL with shared/exclusive locks.
- The existing rollback journal is retained because it already provides atomic pre-image recovery and is covered by process-kill tests. Conversion to a multi-version WAL/checkpoint format is deferred to a later storage-focused phase.
- Cross-process concurrent writers remain unsupported by the embedded file-lock contract; the Phase 2 server will be the single process coordinating network clients.
