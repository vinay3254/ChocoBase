# Phase 1 Design: Concurrency Foundation

## Architecture

The engine keeps its existing `Pager` and executor-facing `&mut Pager` contract. A new `BufferPool` module provides the concurrency primitives that the current pager can adopt incrementally and that shared database handles use today:

- `BufferPool` owns `Frame` objects keyed by page number.
- A frame stores the page bytes, an `RwLock` latch, an atomic pin count, and dirty metadata.
- `PageReadGuard` and `PageWriteGuard` pin a frame for their lifetime and release the pin on drop.
- Eviction is clock/LRU-style over unpinned frames only; attempting to evict a pinned frame returns a capacity error.

The legacy pager remains the persistence authority. `SharedDatabase` wraps a `Database` in an `Arc<Mutex<Database>>` and routes each operation through the lock manager before taking the pager mutex. This avoids exposing references into a concurrently-mutated pager while allowing multiple independent client threads to share one database safely.

## Lock manager

`LockManager` maintains a condition-variable protected table of resource owners. Resources are currently database/table names (page-level logical locking is deferred until the executor has stable row identifiers). `LockMode::Shared` is compatible with other shared owners; `LockMode::Exclusive` is compatible only with itself for the same transaction. Requests wait until compatible. A transaction owns a `LockToken`; dropping the token releases all locks, and explicit `commit`/`rollback` release them deterministically.

The shared handle uses one transaction id per `execute` call unless the SQL statement is `BEGIN`, `COMMIT`, or `ROLLBACK`. Explicit transactions retain their token across statements in that handle. Reads take a shared database lock; mutating statements take an exclusive database lock. This is strict 2PL at the database resource granularity, sufficient to prevent concurrent pager mutation while allowing independent read-only calls to proceed through the shared lock manager.

## Durability

No on-disk format changes. The rollback journal continues to write page pre-images before dirty pages reach the database file, `fsync` the journal before commit, flush dirty pages, and remove the journal. The existing recovery scanner and process-kill tests remain authoritative. A true append-only WAL with checkpoints is explicitly deferred because converting the current pre-image protocol would be an on-disk compatibility decision outside this foundation phase.

## Failure handling

- Guard drops release frame pins even during unwinding.
- Lock-token drops release all owned locks.
- Shared database execution rolls back an in-progress transaction after executor errors, matching the existing embedded behavior.
- No lock is held while waiting for a pager mutex in a way that can create lock-order inversion; resource lock acquisition precedes the short pager critical section and is released after the operation.

## Testing strategy

- Unit tests for frame pin/unpin, read/write latch compatibility, and pinned-frame eviction.
- Lock-manager tests with multiple threads for shared-reader overlap, writer exclusion, and release after drop.
- Integration stress test with several reader threads and writer threads sharing one database handle, asserting no panics, no duplicate primary keys, and a deterministic final row count.
- Existing persistence and crash-recovery suites run unchanged.
