# Phase 6: Operations & Deployment Implementation Plan

## Objective

Make ChocoBase observable, packageable, upgradeable, and operable as a production service with health checks, metrics, containers, migrations, and an administrative CLI.

## Prerequisites

- Server, authentication, REST, and realtime components expose lifecycle and telemetry hooks.
- Database format and catalog versions are identifiable.

## Documentation Gate

Create Phase 6 `requirements.md`, `design.md`, and `tasks.md`. The design must define service-level indicators, health semantics, backup/restore boundaries, migration safety, container persistence, secrets handling, and upgrade/rollback procedures.

## Implementation Work

### Observability

- Add a Prometheus-compatible `/metrics` endpoint on a separately configurable administrative listener.
- Measure connections, active transactions, query latency, errors, lock waits, buffer-pool activity, WAL/journal/checkpoint work, REST requests, WebSocket subscribers, CDC lag, and auth failures.
- Avoid high-cardinality labels such as raw SQL, user IDs, table IDs, or unbounded error strings.
- Add liveness, readiness, and startup health endpoints with distinct semantics.
- Make structured logs correlate requests, connections, transactions, and background jobs.

### Docker Packaging

- Add a multi-stage Docker build with a minimal non-root runtime image.
- Define persistent data, configuration, certificates, and secrets locations.
- Add health checks, signal forwarding, resource limits guidance, and graceful shutdown.
- Provide local development compose configuration only if it does not become the production deployment contract.

### Migration Tooling

- Add ordered, checksummed schema migrations with an internal migration history table.
- Support status, apply, validate, and dry-run commands.
- Acquire an advisory migration lock and fail cleanly when another migration is active.
- Define transactional behavior for DDL and recovery for non-transactional rewrite steps.
- Add format-version checks and block unsafe downgrades.

### Admin CLI

- Add commands for server status, configuration validation, user administration, migration control, checkpoint/maintenance operations, and diagnostic reports.
- Require explicit confirmation for destructive actions and support non-interactive automation flags.
- Use stable machine-readable output alongside human-readable output.
- Keep secrets out of command history and process arguments where possible.

### Operational Safety

- Document backup and restore procedures, including consistency requirements.
- Add startup checks for filesystem permissions, disk space, incompatible versions, and corrupted configuration.
- Define retention for logs, CDC, journals/WAL, and temporary files.
- Add resource-bound tests for file descriptors, memory, connections, and queue sizes.

## Testing

- Metrics format and semantic tests under real workload.
- Health transition tests during startup, shutdown, storage failure, and dependency failure.
- Build and run the Docker image, persist data across container replacement, and verify non-root operation.
- Migration concurrency, checksum mismatch, interrupted migration, upgrade, and downgrade-rejection tests.
- Admin CLI integration tests with machine-readable output.
- Backup/restore and process-kill recovery drills.
- Full end-to-end regression suite across SQL, REST, WebSocket, auth, and persistence.

## Acceptance Criteria

- Operators can distinguish alive, ready, degraded, and failed states.
- Metrics are useful without exposing secrets or causing cardinality explosions.
- Container replacement does not lose committed data.
- Migrations are ordered, auditable, concurrency-safe, and recoverable.
- Administrative operations are scriptable and protect destructive actions.

## Deferred

- Kubernetes operator and automated horizontal scaling.
- Managed cloud control plane, billing, and organization management.
- Multi-region replication and automated failover.
- Full web-based administration console.

