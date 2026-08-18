# Phase 2: Networking & Server Daemon Implementation Plan

## Objective

Turn the Phase 1 concurrent engine into a long-running database service that accepts many network clients while preserving transaction isolation and crash recovery.

## Prerequisites

- Phase 1 buffer pool, lock manager, concurrent database handle, and stress tests are complete.
- The server process is the sole owner of the database file.
- Existing embedded and CLI modes continue to work.

## Documentation Gate

Before implementation, create a Phase 2 directory containing `requirements.md`, `design.md`, and `tasks.md`. The design must record protocol compatibility, connection lifecycle, cancellation, transaction ownership, configuration precedence, and shutdown behavior.

## Architecture Decisions to Validate

- Prefer PostgreSQL wire protocol v3 through a maintained Rust crate such as `pgwire`.
- Document the tradeoff: PostgreSQL compatibility gives immediate driver/tool support but requires translating PostgreSQL protocol concepts and metadata into ChocoBase capabilities.
- Use Tokio for asynchronous sockets and task scheduling.
- Keep blocking storage execution outside Tokio worker threads with `spawn_blocking` or a bounded execution pool.
- Give every connection an isolated session object containing transaction state, authentication placeholder, database selection, and cancellation state.

## Implementation Work

1. Add a dedicated server binary, separate from the existing CLI binary.
2. Add typed configuration loaded in this order: defaults, config file, environment variables, command-line overrides.
3. Add structured logging with connection ID, session ID, transaction ID, latency, result status, and error category.
4. Build a server lifecycle controller supporting startup validation, signal handling, graceful drain, transaction rollback on disconnect, and bounded shutdown timeout.
5. Implement PostgreSQL startup, SSL negotiation response, authentication placeholder, ready-for-query state, simple query flow, error response, row description, data rows, command completion, and termination.
6. Add extended-query protocol support only if required by common clients; otherwise document it as deferred with tested client limitations.
7. Map each network connection to its own ChocoBase session while sharing the Phase 1 engine safely.
8. Enforce maximum connections, query size limits, idle timeout, statement timeout, and bounded request queues.
9. Add protocol-safe error conversion without leaking filesystem paths or internal details.
10. Update packaging and README usage only after the server behavior is stable.

## Testing

- Real TCP client-server tests using at least one PostgreSQL client library.
- Concurrent connection stress test with readers, writers, explicit transactions, disconnects, and failed statements.
- Graceful shutdown test proving active work drains and unfinished transactions roll back.
- Process-kill recovery test against the server binary.
- Configuration precedence and invalid-configuration tests.
- Protocol malformed-input, oversized-message, timeout, and connection-limit tests.
- Full Phase 1 regression suite.

## Acceptance Criteria

- Common PostgreSQL clients can connect and run supported SQL.
- Many connections can execute without blocking the async runtime.
- Connection loss never leaves locks or transactions active.
- Shutdown and crash recovery preserve committed data and discard uncommitted work.
- Unsupported protocol features return clear protocol errors rather than disconnecting unpredictably.

## Deferred

- Production credential authentication and authorization, reserved for Phase 4.
- TLS termination unless required for safe testing; production TLS may be placed behind a proxy initially.
- Connection pooling and read replicas.

