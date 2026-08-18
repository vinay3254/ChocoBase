# Phase 5: Auto-Generated APIs & Realtime Implementation Plan

## Objective

Expose authenticated database tables through schema-driven REST endpoints and deliver committed row changes to WebSocket subscribers while applying the same RLS rules as SQL queries.

## Prerequisites

- Phase 4 authentication claims and policy enforcement are centralized and reusable.
- Phase 3 catalog metadata accurately describes columns, types, constraints, and relationships.

## Documentation Gate

Create Phase 5 `requirements.md`, `design.md`, and `tasks.md`. The design must define REST mapping rules, API versioning, transaction boundaries, CDC durability, event ordering, resume behavior, backpressure, and how RLS is evaluated for subscriptions and emitted rows.

## Implementation Work

### REST API

- Add an HTTP server integrated with the existing daemon lifecycle and configuration.
- Generate table CRUD routes from catalog metadata.
- Support column selection, equality/range filters, ordering, pagination, and bounded result sizes.
- Translate JSON request values through the Phase 3 type system.
- Execute every request with Phase 4 authenticated claims and normal transaction semantics.
- Return stable error envelopes without leaking SQL or internal paths.
- Cache schema descriptions with catalog-version invalidation.

### Change Data Capture

- Add a transaction-aware change collector for insert, update, and delete operations.
- Publish events only after commit; discard them on rollback.
- Include transaction ID, commit sequence, table identity, operation, primary key, and permitted old/new row data.
- Define a bounded durable event log or explicitly document realtime as ephemeral.
- Preserve event order within a transaction and define cross-transaction ordering.

### WebSocket Realtime

- Add authenticated WebSocket upgrade and subscription messages.
- Support table, operation, and approved column filters.
- Evaluate Phase 4 RLS against each subscriber's claims before delivery.
- Re-evaluate authorization when policies or user status change.
- Add heartbeat, disconnect detection, bounded subscriber queues, slow-consumer policy, and connection limits.
- Support resume cursors if CDC storage is durable; otherwise clearly signal missed-event windows.

## Testing

- Real HTTP tests for CRUD, filters, pagination, constraints, types, and transaction errors.
- Cross-check REST results against equivalent SQL queries.
- Real WebSocket tests with multiple authenticated tenants and concurrent changes.
- RLS isolation tests proving unauthorized rows never appear in REST responses or realtime events.
- Commit/rollback tests proving only committed changes emit events.
- Slow-consumer, reconnect, malformed-message, connection-limit, and backpressure tests.
- Process-kill tests for durable CDC if durability is shipped.
- Full earlier-phase regression suites.

## Acceptance Criteria

- New tables become available through the REST surface without handwritten handlers.
- REST and WebSocket requests reuse the same authentication and RLS enforcement path as SQL.
- Rolled-back changes never emit events.
- Subscriber overload cannot exhaust server memory.
- CDC ordering and delivery guarantees are explicitly documented and tested.

## Deferred

- GraphQL generation.
- Arbitrary server-side functions and triggers over HTTP.
- Exactly-once event delivery; at-least-once or best-effort semantics must be documented honestly.
- Cross-region realtime fan-out.

