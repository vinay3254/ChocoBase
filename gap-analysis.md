# ChocoBase Gap Analysis: Embedded Storage Engine vs. Multi-Tenant Platform

This document presents an architectural and capability audit of the current **ChocoBase** codebase compared against the requirements for a networked, multi-tenant database platform (Supabase/Firebase-class).

---

## 1. Concurrency Model

### Current State
* **Single-Process Exclusive Locking (`src/storage/lock.rs`):** An advisory lock file (`<database>.lock`) holding the active process PID is required to open or create a database. Simultaneous opens by separate processes fail immediately with `StorageError::DatabaseLocked`.
* **Single-Threaded Pager (`src/storage/pager.rs`):** The `Pager` struct owns an in-memory page cache (`HashMap<u32, Page>`) and dirty set (`HashSet<u32>`) without thread-synchronization primitives (`Arc`, `RwLock`, `Mutex`).
* **Exclusive Engine Access (`src/engine.rs`):** The `Database` struct requires `&mut self` for all query execution (`Database::execute`), metadata introspection (`table_schema`, `list_tables`), and transaction lifecycle methods.
* **No Reader Concurrency or MVCC:** Even read-only `SELECT` queries require exclusive mutable ownership. Mutations overwrite pages in-place in memory after saving pre-images to the rollback journal; there is no multi-version snapshot isolation.

### What Is Missing
* **Concurrent Readers:** Ability for multiple read queries to execute in parallel without blocking one another.
* **Thread-Safe Buffer Pool:** Replacement of single-threaded `Pager` with a frame-based buffer pool manager using latching (`RwLock` on page frames) and pin/unpin reference counting.
* **Multi-Version Concurrency Control (MVCC) / Lock Manager:** Row-level or page-level shared/exclusive locks (2PL) or tuple-versioned snapshot isolation to allow concurrent readers and writers.
* **Connection Multiplexing:** Worker pool or async task scheduling to serve multiple client sessions against a shared storage engine.

### Complexity to Close Gap
* **Large (1–2 months):** Requires rewriting the storage pager into a multi-threaded latching buffer pool manager and introducing a transaction lock manager or MVCC version store.

---

## 2. Network Layer

### Current State
* **Embedded Library & CLI Binary Only (`src/main.rs`, `src/lib.rs`):** The engine is consumed either as an in-process Rust library (`dbengine::Database`) or via a local interactive CLI REPL executable (`src/main.rs`, `src/repl/mod.rs`).
* **Zero Network Exposure:** There is no socket listener, networking dependency, or RPC server in the codebase.

### What Is Missing
* **Transport Protocols:** TCP/TLS listener and connection acceptor.
* **Database Wire Protocol:** Implementation of a standard wire protocol (e.g. PostgreSQL v3 wire protocol `pgwire` or MySQL protocol) or a custom framed binary protocol.
* **Async I/O Runtime:** Integration of an asynchronous runtime (`tokio`) to handle concurrent client sockets, request framing, authentication handshakes, and response streaming.
* **Client Drivers / SDKs:** Client libraries for languages (TypeScript/JS, Python, Go) to connect over the network.

### Complexity to Close Gap
* **Medium (2–4 weeks):** Integrating Tokio and a wire protocol server (such as `pgwire`) on top of an engine interface.

---

## 3. Schema & Query Surface

### Current State
* **SQL Parsing & AST (`src/sql/lexer.rs`, `src/sql/parser.rs`, `src/sql/ast.rs`):** Supports a clean, functional subset of single-table SQL:
  * `CREATE TABLE <table> (<col> <type> [NOT NULL] [PRIMARY KEY])` (strictly 1 PK required).
  * `DROP TABLE <table>`.
  * `CREATE INDEX <name> ON <table> (<col>)` (single-column B+Tree index on non-null column).
  * `DROP INDEX <name>`.
  * `INSERT INTO <table> [(<cols>)] VALUES (...), (...)`.
  * `SELECT <cols|*> FROM <table> [WHERE <expr>] [ORDER BY <col> [ASC|DESC]] [LIMIT <n>]`.
  * `UPDATE <table> SET <col> = <expr>, ... [WHERE <expr>]`.
  * `DELETE FROM <table> [WHERE <expr>]`.
  * `BEGIN [TRANSACTION]`, `COMMIT [TRANSACTION]`, `ROLLBACK [TRANSACTION]`.
* **Query Planner & Executor (`src/plan/planner.rs`, `src/exec/`):**
  * Cost-heuristic seek selection for indexed/PK equality and range scans.
  * Streaming volcano-style iterator operators (`Scan`, `IndexScan`, `Filter`, `Project`, `Sort`, `Limit`, `Mutate`).
* **Permissions / Roles:** Zero schema-level users, roles, or grants.

### What Is Missing
* **Relational Joins:** `INNER JOIN`, `LEFT/RIGHT/FULL OUTER JOIN`, `CROSS JOIN`, hash join / merge join execution operators.
* **Aggregations & Grouping:** `COUNT`, `SUM`, `AVG`, `MIN`, `MAX`, `GROUP BY`, `HAVING`.
* **Advanced Query Syntax:** Subqueries (`IN`, `EXISTS`, scalar subqueries), CTEs (`WITH`), Set operations (`UNION`, `INTERSECT`, `EXCEPT`).
* **Constraints & Defaults:** `FOREIGN KEY` (referential integrity), `UNIQUE` secondary constraints, `CHECK` expressions, `DEFAULT` value expressions, `AUTO_INCREMENT` / sequences.
* **Schema Evolution:** `ALTER TABLE` (add/drop/rename columns, change types).
* **Row-Level Security (RLS) & ACLs:** `CREATE ROLE`, `GRANT/REVOKE`, `CREATE POLICY ... ON <table> FOR SELECT USING (...)` (indispensable for client-direct platforms like Supabase/Firebase).

### Complexity to Close Gap
* **Large to Multi-Month:** Joins, aggregations, and planner optimizer (1–2 months); Full RLS policy evaluator and schema migration engine (1–2 months).

---

## 4. Durability & Recovery

### Current State (Built & Verified)
* **Pre-Image Rollback Journal (`src/storage/journal.rs`):**
  * 64-byte structured header (`CHOCOJNL` magic, version 1, header CRC32, `orig_page_count`).
  * 4104-byte streaming undo records (`page_no`, 4096-byte raw data, trailing CRC32).
  * Automatic `fsync` barriers on journal creation and before live database page mutation.
* **Crash Recovery Algorithm (`recover_if_needed`):**
  * Automatic detection of `<db>-journal` on `Database::open()`.
  * Sequential scan-until-checksum-fail/EOF strategy: stops at the first bad/truncated record and safely undoes the verified pre-image prefix into the database file.
  * File truncation back to `orig_page_count`, followed by disk sync and journal deletion.
* **Transaction Controls (`src/storage/pager.rs`, `src/engine.rs`):**
  * Full rollback of DML (`INSERT`, `UPDATE`, `DELETE`) and DDL (`CREATE/DROP TABLE`, `CREATE/DROP INDEX`).
  * Automatic single-statement autocommit wrapping.
  * Process crash resistance validated via child process termination (`SIGKILL`) integration tests (`tests/crash_recovery.rs`).

### What Is Missing (For Cloud / Multi-Tenant Scale)
* **Write-Ahead Logging (WAL) Architecture:** The rollback journal requires synchronous in-place writes to the main DB file at commit time. A WAL with asynchronous background checkpointing is required for high-throughput concurrent writers and non-blocking readers.
* **Point-In-Time Recovery (PITR):** Continuous WAL archiving to cloud storage (S3/GCS) with replay to arbitrary timestamps.
* **Replication & Consensus:** Raft or primary-replica physical/logical replication streams.

### Complexity to Close Gap
* **Medium (3–4 weeks)** to migrate rollback journal to WAL + checkpointing.
* **Large (1–2 months)** for continuous cloud WAL streaming and PITR.

---

## 5. Data Model Ceiling

### Current State
* **Primitive Scalar Types (`src/types/value.rs`, `src/types/schema.rs`):**
  * `Integer` (`i64`) — 8-byte sign-flipped order-preserving binary encoding.
  * `Text` (`String`) — Null-terminated UTF-8 byte encoding (must fit in single page payload limit).
  * `Boolean` (`bool`) — 1-byte encoding (`0` / `1`).
  * `Null`.
* **No Overflow / Large Object Pages:** Rows are constrained to fit within a single 4KB page (`PAGE_SIZE - header`); oversized rows return `StorageError::RowTooLarge`.

### What Is Missing
* **Complex / Cloud-Standard Types:**
  * Floating-point & Fixed-precision: `REAL`, `DOUBLE PRECISION`, `NUMERIC(p,s)`.
  * Date & Time: `DATE`, `TIME`, `TIMESTAMP`, `TIMESTAMPTZ`, `INTERVAL`.
  * Binary / Large Objects: `BLOB`, `BYTEA`, overflow / TOAST pages for strings/binaries exceeding page size.
  * Semi-Structured Data: `JSON` / `JSONB` with JSON path operators (`->`, `->>`, `@>`) and containment queries.
  * Arrays: `INTEGER[]`, `TEXT[]`.
  * Identifiers: `UUID` v4 / v7.
* **Specialized Indexing Structures:**
  * GIN / Inverted Indexes for JSON document querying and Full-Text Search (`tsvector`, `tsquery`).
  * Vector Indexes (HNSW / IVF-Flat) for semantic similarity search (AI embeddings).

### Complexity to Close Gap
* **Large (1–2 months):** TOAST overflow storage mechanism, JSON document indexing, and extended type system.

---

## 6. Auth / Identity

### Current State
* **Explicitly Absent:** There are zero concepts of users, passwords, sessions, authentication tokens, API keys, cryptographic signatures, or tenant identifiers in the entire codebase.

### What Is Missing
* **Authentication Subsystem:** User registration, password hashing (Argon2id/bcrypt), MFA, OAuth2 / OIDC providers, magic links (equivalent to Supabase GoTrue / Firebase Auth).
* **Token Verification & Claims:** JWT signing and validation, extracting tenant IDs and user IDs (`auth.uid()`) to feed into query execution contexts.
* **Multi-Tenant Isolation Models:**
  * *Option A (Shared Database, Tenant Key):* Automatically injecting `WHERE tenant_id = current_tenant()` into query ASTs.
  * *Option B (Database-Per-Tenant):* Dynamic connection router opening distinct database files per tenant.

### Complexity to Close Gap
* **Large (1–2 months):** Either integrating an external identity service (e.g. GoTrue) and connecting claims to query context, or building an internal identity management service.

---

## 7. External Interfaces & Realtime

### Current State
* **Explicitly Absent:** No HTTP/REST generation, no GraphQL engine, no webhook triggers, and no pub-sub or change notifications.

### What Is Missing
* **Auto-Generated Data APIs:**
  * HTTP REST API mapping tables/views to CRUD endpoints with query filtering (`PostgREST` equivalent).
  * GraphQL API auto-generating queries/mutations from database schemas (`pg_graphql` equivalent).
* **Realtime Change Data Capture (CDC):**
  * Engine-level logical replication / mutation changefeed emitting events on `INSERT`, `UPDATE`, `DELETE`.
  * WebSocket / SSE server broadcasting table change events to connected client applications filtered by tenant and RLS policy (Supabase Realtime / Firebase Firestore onSnapshot).
* **Database Webhooks:** Outgoing asynchronous HTTP dispatchers on mutation events.

### Complexity to Close Gap
* **Large to Multi-Month (2–3 months):** Building a CDC stream, WebSocket distribution server, and auto-generated REST/GraphQL gateway.

---

## 8. Deployment Shape

### Current State
* **Library + Interactive CLI (`src/lib.rs`, `src/main.rs`):**
  * Invoked as `dbengine <path_to_db>`.
  * Runs single process in the foreground attached to stdin/stdout.
  * Process lifecycle ends when the user exits the REPL or the hosting application terminates.

### What Is Missing
* **Server Daemon Binary:** Dedicated daemon process lifecycle (`chocod` / `chocobase-server`) running as a system service or container.
* **Configuration Management:** Configuration files (`chocobase.toml`), environment variable overrides, CLI arguments for network bind host/port, storage paths, memory budgets, and connection limits.
* **Observability & Operations:**
  * Structured logging (`tracing`) and metrics exporter (`/metrics` for Prometheus).
  * Graceful shutdown handling (draining connections, flushing buffer pools).
  * Admin dashboard / UI console.

### Complexity to Close Gap
* **Small to Medium (1–2 weeks):** Standard server binary scaffolding, configuration loader, and daemon lifecycle.

---

## Summary

| Category | ChocoBase Current State | Multi-Tenant Platform Requirement | Gap Complexity |
| :--- | :--- | :--- | :--- |
| **Concurrency** | Single-process, single-writer exclusive lock (`.lock`), `&mut self` execution | Latching buffer pool, concurrent readers, MVCC / Lock Manager | **Large (1–2 mo)** |
| **Networking** | Embedded library + CLI REPL only; no socket layer | TCP/TLS listener, async runtime (`tokio`), wire protocol (`pgwire`) | **Medium (2–4 wks)** |
| **Schema & SQL** | Single-table CRUD, B+Tree secondary index, simple `WHERE/ORDER/LIMIT` | Joins, aggregations, subqueries, FKs, RLS policies, `ALTER TABLE` | **Multi-Month (2–3 mo)** |
| **Durability** | Crash-safe Rollback Journal, undo replay, CRC32, autocommit | WAL + checkpointing, cloud PITR streaming, replication | **Medium to Large** |
| **Data Model** | `INTEGER` (i64), `TEXT`, `BOOLEAN`, `NULL` (single-page rows) | Float, Date/Time, JSONB, Arrays, UUID, TOAST overflow, Vector | **Large (1–2 mo)** |
| **Auth & Identity** | None | User auth (JWT/OAuth), role permissions, tenant isolation | **Large (1–2 mo)** |
| **External Interfaces** | None | PostgREST-style REST, GraphQL, WebSocket Realtime CDC | **Multi-Month (2–3 mo)** |
| **Deployment Shape**| In-process library & CLI binary | Configured standalone daemon server with metrics/observability | **Small to Medium (1–2 wks)** |

---

### Concluding Assessment

ChocoBase is currently a self-contained, embedded single-writer SQL storage engine with page-level B+Tree indexing, rollback journaling, and atomic transaction recovery — architecturally comparable to an early-stage embedded storage engine like SQLite in rollback mode. In contrast, platforms like Supabase and Firebase are distributed multi-tier application platforms comprising a network wire protocol server, multi-tenant authentication and authorization infrastructure, row-level security policy engines, auto-generated REST/GraphQL interfaces, and a realtime change-data-capture (CDC) pub-sub broadcasting layer.
