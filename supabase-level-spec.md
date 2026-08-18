# ChocoBase Supabase-Class Platform

## Final Product Specification and Master Implementation Plan

**Document status:** DRAFT - EXECUTION NOT AUTHORIZED  
**Implementation status:** Planning only  
**Execution gate:** No phase in this document may begin until the repository owner explicitly authorizes that phase.  
**Last updated:** 2026-08-18  
**Target:** A secure, production-grade, self-hostable backend platform with capabilities comparable to the major product surfaces of Supabase.

---

## 1. Purpose

This document is the single authoritative specification and implementation plan for evolving ChocoBase from a custom Rust database prototype into a Supabase-class backend platform.

It replaces the fragmented phase plans as the forward-looking product contract. Existing phase documents remain historical references, but this document controls future architecture, sequencing, acceptance criteria, release gates, and definitions of done.

The target is not a visual imitation or a collection of endpoints with Supabase-like names. The target is a coherent platform whose security boundaries, durability, tenant isolation, operational behavior, and developer workflows are reliable enough for real applications.

---

## 2. Planning Rules

1. No implementation work starts from this document without explicit owner approval.
2. Approval must name a phase or a bounded work package.
3. A phase may not be marked complete because its happy-path tests pass. All phase exit criteria and release gates must pass.
4. Security-sensitive behavior must use established libraries and protocols. Cryptography, password hashing, token formats, HTTP parsing, TLS, database wire protocols, OAuth, WebSockets, and object-storage signing must not be hand-rolled.
5. Existing passing behavior must remain covered by regression tests unless this specification explicitly replaces it.
6. Unsupported behavior must fail closed and return a stable error. It must not silently become administrator access, skip RLS, lose committed data, or expose another tenant's data.
7. Documentation, configuration examples, containers, and tests must describe the behavior actually shipped.
8. Every production-facing feature must define resource limits, timeouts, observability, failure behavior, upgrade behavior, and abuse controls.

---

## 3. Product Definition

ChocoBase will become a platform that gives application developers:

- A durable relational database accessible through PostgreSQL-compatible clients.
- Authentication, session management, and identity administration.
- Database-enforced row-level security and role-based authorization.
- Auto-generated REST and GraphQL data APIs.
- Realtime database changes, broadcast channels, and presence.
- S3-compatible object storage with access policies and signed URLs.
- Serverless or edge-style functions with managed secrets and logs.
- A web Studio for database, auth, storage, functions, logs, and project settings.
- A CLI and local development stack.
- Backups, point-in-time recovery, replication, monitoring, audit logs, and controlled upgrades.
- Client SDKs that expose a consistent application-facing API.

The platform must support both self-hosted single-project installations and a future managed multi-project control plane.

---

## 4. Compatibility Contract

### 4.1 Supabase-Class, Not Trademark Compatibility

The project will provide equivalent product categories and developer workflows without claiming official Supabase compatibility, certification, or affiliation.

### 4.2 PostgreSQL Compatibility

PostgreSQL compatibility is a core product requirement because it unlocks mature drivers, ORMs, migration tools, BI tools, SQL editors, and ecosystem expectations.

The final target includes:

- PostgreSQL wire protocol v3.
- Simple and extended query protocols.
- Prepared statements and parameter binding.
- Transaction status, cancellation, error fields, and type metadata.
- A documented SQL compatibility matrix.
- Common PostgreSQL scalar types and semantics.
- Migration-tool compatibility for selected tools.
- Compatibility tests against `psql`, common Rust clients, Node PostgreSQL clients, and at least two mainstream ORMs.

Full compatibility with every PostgreSQL extension, system catalog, query planner behavior, and edge-case SQL semantic is not required for the first production release. Unsupported features must be documented and rejected clearly.

### 4.3 Architecture Decision Gate

Before Phase 2 begins, the owner must choose one database strategy:

#### Strategy A: PostgreSQL-Backed Platform - Recommended for Fastest Product Delivery

- Use PostgreSQL as the database kernel.
- Build ChocoBase services around PostgreSQL for Auth, API, Realtime, Storage, Functions, Studio, CLI, and control-plane behavior.
- Preserve the current custom engine as an experimental or embedded product.
- Expected advantage: dramatically lower risk for SQL, MVCC, WAL, replication, drivers, migrations, backup tooling, and extensions.
- Expected disadvantage: ChocoBase is no longer a from-scratch database at the center of the hosted platform.

#### Strategy B: ChocoBase-Native Platform - Maximum Ownership, Maximum Scope

- Continue developing the custom engine into the production database kernel.
- Implement PostgreSQL wire and a documented PostgreSQL-compatible subset.
- Build MVCC, WAL, replication, query semantics, constraints, types, backups, and operational tooling in-house.
- Expected advantage: full architectural ownership and differentiation.
- Expected disadvantage: multi-year database-engine work before reaching mature production reliability.

The remaining plan applies to both strategies. Database-kernel tasks marked **Native only** are unnecessary when Strategy A is selected.

---

## 5. Current Baseline

The current repository provides a useful prototype baseline:

- Page-based persistent storage and B+ tree tables/indexes.
- Rollback-journal crash recovery.
- Basic transactions.
- A limited SQL parser, planner, and executor.
- Inner/left joins and basic aggregations.
- JSON validation and extraction.
- A shared database facade and logical lock manager.
- A custom TCP JSON protocol.
- Basic change-event broadcasting.
- Prototype users, tokens, RLS, REST CRUD, HTTP dashboard, and migrations.
- A release binary and draft container files.

These features are not accepted as production implementations. In particular, authentication, JWT signing, unauthenticated administrator behavior, TCP authorization, container startup, true reader concurrency, HTTP parsing, and operational controls must be replaced or redesigned before public exposure.

---

## 6. Guiding Architecture

### 6.1 Service Boundaries

The production platform will be divided into independently testable modules or services:

1. **Database service** - relational execution, transactions, storage, SQL protocol, RLS, replication, and backups.
2. **API gateway** - REST, GraphQL, RPC, request authentication, limits, and response shaping.
3. **Auth service** - identities, credentials, OAuth/OIDC, sessions, MFA, email/SMS flows, and admin APIs.
4. **Realtime service** - database changes, broadcast, presence, subscriptions, and fan-out.
5. **Storage service** - buckets, object metadata, uploads, downloads, transformations, and signed URLs.
6. **Functions service** - deploy, isolate, execute, meter, and observe user functions.
7. **Studio** - browser-based project administration.
8. **Control plane** - organizations, projects, provisioning, domains, secrets, versions, billing hooks, and fleet operations.
9. **Observability plane** - logs, metrics, traces, audit events, alerts, and diagnostic bundles.

The first releases may run these modules in one process or one Compose stack, but internal boundaries and contracts must allow later separation.

### 6.2 Data Plane and Control Plane

- The **data plane** handles application traffic and project data.
- The **control plane** manages project lifecycle and configuration.
- A control-plane failure must not automatically take healthy data-plane projects offline.
- Control-plane credentials must not be accepted as ordinary project data credentials.
- Project secrets must be encrypted and scoped to a single project.

### 6.3 Trusted Boundaries

- Public clients are untrusted.
- JWT claims are untrusted until signature and claim validation succeeds.
- SQL, REST filters, GraphQL documents, filenames, object metadata, migration files, and function bundles are untrusted input.
- Studio is not a trusted bypass merely because it is first-party UI.
- Administrative bypasses require explicit service credentials, auditable actions, and narrowly scoped permissions.

---

## 7. Database Platform Specification

### 7.1 Transaction and Concurrency Model

The database must provide:

- Atomicity, consistency, isolation, and durability for supported operations.
- Concurrent readers that do not serialize behind a single process-wide mutex.
- Multiple concurrent transactions.
- Snapshot isolation or stronger semantics for normal application workloads.
- Explicit transaction states and isolation levels.
- Deadlock detection or prevention with deterministic victim handling.
- Statement cancellation and transaction abort semantics.
- Transaction-scoped change collection for Realtime and audit hooks.
- Connection-local session state.

**Native only:** Replace the global mutable database mutex with an integrated concurrent buffer manager and MVCC transaction manager. A standalone buffer-pool module that is not used by the pager does not satisfy this requirement.

### 7.2 WAL, Recovery, and Checkpoints

The production database must provide:

- Write-ahead logging with checksummed records.
- Redo recovery after unclean shutdown.
- Transaction commit records and durable ordering.
- Background checkpoints.
- WAL retention policies.
- Recovery from torn/truncated final records.
- Crash tests at every durability boundary.
- A documented fsync model for supported operating systems and filesystems.
- Corruption detection with safe startup failure.

Rollback journaling may remain available for embedded mode but is not the target cloud transaction architecture.

### 7.3 Replication and High Availability

The GA target includes:

- One primary and at least one replica.
- Physical or logical replication with ordered log positions.
- Replica catch-up and lag metrics.
- Controlled promotion.
- Fencing to prevent split brain.
- Rejoin or rebuild procedures for old primaries.
- Read-only replica connections.
- Backup integration with the replication log.

Automated cross-region consensus is not required for the first GA release, but architecture must not prevent it.

### 7.4 SQL Surface

Required SQL capabilities include:

- `CREATE`, `ALTER`, and `DROP` for schemas, tables, indexes, views, and supported functions.
- `INSERT`, `UPDATE`, `DELETE`, `SELECT`, and upsert behavior.
- Inner, left, right, full, and cross joins.
- Aggregates, `GROUP BY`, `HAVING`, window functions, and ordered aggregates.
- Subqueries, common table expressions, recursive CTEs, set operations, and `RETURNING`.
- Prepared statements and typed parameters.
- Transactions, savepoints, and rollback to savepoint.
- `EXPLAIN` and `EXPLAIN ANALYZE` with stable machine-readable output.
- Identifier quoting, schemas, aliases, casts, and SQL three-valued null semantics.
- Sequences or identity columns.
- Generated columns where feasible.

### 7.5 Types

Required first-class types include:

- Signed integers of common widths.
- Floating-point values.
- Fixed-precision numeric/decimal.
- Boolean.
- Text and bounded character types.
- Binary data.
- UUID.
- Date, time, timestamp, timestamp with time zone, and interval.
- JSON and binary/canonical JSON representation.
- Arrays for supported element types.
- Network address types if PostgreSQL compatibility requires them.
- Vector embeddings with a documented dimension limit.
- Null.

Values larger than one page require overflow or large-object storage. Row encoding must be versioned and migration-safe.

### 7.6 Constraints and Referential Integrity

The database must enforce:

- Primary keys.
- Unique constraints, including composite uniqueness.
- Not-null constraints.
- Check constraints.
- Defaults.
- Foreign keys with `RESTRICT`, `NO ACTION`, `CASCADE`, `SET NULL`, and `SET DEFAULT` where supported.
- Deferred constraints where claimed.
- Constraint correctness under concurrent transactions.

### 7.7 Indexes and Search

Required index capabilities include:

- Composite B-tree indexes.
- Unique indexes.
- Partial indexes.
- Expression indexes.
- Index-only scans where storage visibility permits.
- JSON containment/search indexing.
- Full-text search.
- Vector similarity indexes such as HNSW or IVF-based indexing.
- Online or minimally blocking index creation for production-sized tables.

### 7.8 Query Planning and Resource Governance

- Cost-based access-path selection.
- Collected table/index statistics.
- Join-order planning.
- Bounded-memory sorts, joins, and aggregations with disk spill.
- Per-query memory budgets.
- Statement timeout and cancellation.
- Maximum result and intermediate sizes.
- Slow-query logging with normalized fingerprints.
- Plan regression tests.

### 7.9 PostgreSQL Wire Protocol

- TLS negotiation.
- Startup parameters.
- Authentication exchange.
- Simple query protocol.
- Parse, bind, describe, execute, sync, and close flows.
- Prepared statement lifecycle.
- Parameter and row type metadata.
- ErrorResponse fields with stable SQLSTATE values.
- ReadyForQuery transaction status.
- Cancellation requests.
- Copy-in/copy-out support before GA.
- Connection and statement limits.

The custom newline-delimited JSON protocol may remain as an internal diagnostic protocol but must not be the primary external database interface.

### 7.10 RLS and Database Authorization

- Roles and memberships.
- Grants and revokes for schemas, tables, sequences, functions, and supported objects.
- Table owners.
- `CREATE`, `ALTER`, and `DROP POLICY`.
- Policies scoped to `SELECT`, `INSERT`, `UPDATE`, `DELETE`, or all commands.
- `USING` and `WITH CHECK` expressions.
- Default deny when RLS is enabled and no policy grants access.
- Central enforcement across scans, indexes, joins, subqueries, REST, GraphQL, Realtime, and Storage metadata.
- Narrow, explicit service-role bypass.
- Policy recursion detection.
- Audit events for policy changes and privileged bypass.

---

## 8. Authentication and Identity Specification

### 8.1 Core Identity Model

Each user record must support:

- Stable UUID identifier.
- Project/tenant ownership.
- Email and/or phone identifiers.
- Normalized identifier values and verified timestamps.
- User metadata and application metadata with separate write permissions.
- Account state: active, disabled, banned, deleted, or pending.
- Credential and session versioning.
- Created, updated, last-login, and deletion timestamps.

### 8.2 Password Security

- Argon2id through a maintained implementation.
- Cryptographically secure random salts.
- Versioned parameters and rehash-on-login.
- Constant-time verification provided by the library.
- Password length and breached-password policy hooks.
- No plaintext password logging, tracing, analytics, or persistence.
- Rate limiting and progressive backoff.

### 8.3 Tokens and Sessions

- Standards-compliant JWT access tokens.
- Required validation of algorithm, key ID, signature, issuer, audience, subject, expiry, not-before, and issued-at.
- Asymmetric signing preferred for hosted deployments, with JWKS publication and rotation.
- Short-lived access tokens.
- Rotating refresh tokens stored as hashed secrets.
- Refresh-token reuse detection and session-family revocation.
- Per-device session visibility and logout.
- Administrative user/session revocation.
- Clock-skew policy.
- Secure cookie mode for browser applications.

### 8.4 Sign-Up and Sign-In Methods

- Email/password.
- Phone/password where enabled.
- Email magic link.
- Email OTP.
- SMS OTP through a provider abstraction.
- OAuth 2.0/OIDC providers.
- Anonymous users with controlled account linking.
- Enterprise SSO/SAML as a post-GA enterprise module.

Public sign-up must never accept an administrator or service role supplied by the client.

### 8.5 Account Lifecycle

- Email and phone verification.
- Password reset.
- Email-change confirmation.
- Account linking and unlinking.
- Account deletion and configurable retention.
- Identity conflict resolution.
- Admin invite flows.
- Hooks for custom email and SMS delivery.

### 8.6 Multi-Factor Authentication

- TOTP enrollment and verification.
- Recovery codes.
- MFA challenge state and assurance-level claims.
- Session upgrade after successful challenge.
- Administrative reset with audit trail.
- WebAuthn/passkeys as a later compatible extension.

### 8.7 Abuse Protection

- Per-IP, per-identifier, per-project, and per-route rate limits.
- CAPTCHA/provider hooks.
- Enumeration-resistant responses where appropriate.
- Credential-stuffing signals.
- Configurable sign-up restrictions.
- Disposable-email and domain policy hooks.
- Audit logs for security events.

### 8.8 Auth APIs

- Stable versioned public API.
- Administrative API requiring service credentials.
- OpenAPI specification.
- Idempotency for appropriate mutation endpoints.
- Consistent error codes.
- Webhook and hook retry semantics.
- Backward-compatible token claims during key and schema rotation.

---

## 9. Data API Specification

### 9.1 REST API

The REST API must provide schema-driven routes for exposed tables, views, and approved functions.

Required capabilities:

- Select specific columns.
- Filter with equality, inequality, ranges, null tests, set membership, pattern matching, full-text, JSON paths, and logical composition.
- Order by multiple columns.
- Offset and keyset pagination.
- Count modes.
- Insert one or many rows.
- Update and delete with filters.
- Upsert with conflict targets.
- `RETURNING`/representation preferences.
- Embedded related resources through foreign-key metadata.
- RPC for approved database functions.
- Content negotiation and stable JSON encoding.
- Bounded response sizes and server-enforced maximum limits.
- ETags or cache validators where semantically safe.

All values must be converted through typed parameters. User input must never be concatenated into SQL strings.

### 9.2 GraphQL API

- Schema generation from database metadata.
- Queries, mutations, filtering, ordering, pagination, aggregates, and relationships.
- RLS enforcement through the database execution identity.
- Query depth, complexity, timeout, and result limits.
- Introspection configuration.
- Stable handling of schema changes.
- Persisted-query support as a later optimization.

### 9.3 API Security

- Anonymous key and service-role key separation.
- Service-role keys must never be safe for browser use.
- JWT verification through the Auth key set.
- Request identity propagated to database session claims.
- CORS allowlists rather than unconditional wildcard production defaults.
- CSRF defenses for cookie-authenticated flows.
- Request-size and decompression limits.
- HTTP timeouts and slow-client protection.
- Standard HTTP parser/framework instead of a one-read custom parser.

### 9.4 API Observability

- Request ID and trace context.
- Route template, status, latency, response size, and error category metrics.
- No raw tokens, passwords, or unrestricted request bodies in logs.
- Slow request and database timing breakdown.

---

## 10. Realtime Specification

### 10.1 Product Modes

Realtime must support:

1. **Database Changes** - committed insert, update, and delete events.
2. **Broadcast** - low-latency client messages within authorized channels.
3. **Presence** - synchronized online-state metadata for channel members.

### 10.2 Database Change Capture

- Events originate from committed transaction/WAL order.
- Rolled-back changes never emit.
- Events include project, schema, table, operation, primary key, commit position, transaction ID, and approved old/new data.
- Per-table replica identity requirements are documented.
- Delivery is ordered by commit position within a project.
- Durable resume cursor or an explicit documented best-effort mode.
- Retention and replay windows.
- Backfill behavior when consumers reconnect.

### 10.3 Authorization

- Authenticated WebSocket handshake.
- Channel authorization hooks.
- RLS-aware database-change delivery.
- Policy changes and user revocation eventually terminate or reauthorize subscriptions.
- Service-role subscriptions are separately audited.
- Presence and broadcast payload validation.

### 10.4 Reliability and Scale

- Heartbeats and idle timeout.
- Bounded per-subscriber queues.
- Slow-consumer disconnect or drop policy with explicit notification.
- Horizontal fan-out through a broker or replicated log.
- Connection quotas.
- Payload size limits.
- Reconnect and resume behavior in SDKs.
- Metrics for connections, channels, lag, dropped messages, and delivery latency.

---

## 11. Object Storage Specification

### 11.1 Storage Model

- Projects contain buckets.
- Buckets are public or private.
- Objects have bucket, path, owner, size, content type, checksum, version, and metadata.
- Object metadata is transactional in the database.
- Binary content is stored through a pluggable S3-compatible backend or local development backend.

### 11.2 APIs

- Create, update, list, and delete buckets.
- Upload, download, move, copy, list, and delete objects.
- Multipart/resumable uploads.
- Range requests.
- Signed upload and download URLs.
- Cache-control and content-disposition support.
- Idempotent retries.
- Object versioning as a post-MVP feature.

### 11.3 Authorization

- Storage metadata is protected by database RLS.
- Object operations map to authenticated database checks.
- Public buckets expose reads only according to documented rules.
- Signed URLs contain expiry and bounded operation scope.
- Path normalization prevents traversal and ambiguous encodings.
- Service credentials are scoped and audited.

### 11.4 Image Processing

- Optional resize, crop, format, and quality transforms.
- Resource and pixel-count limits.
- Cached derivatives.
- Safe decoder libraries and decompression-bomb protection.

### 11.5 Integrity and Lifecycle

- Checksums verified on upload.
- Atomic metadata/content publication.
- Garbage collection for abandoned multipart uploads.
- Configurable object retention and deletion lifecycle.
- Backup and restore coordination between metadata and object backend.

---

## 12. Functions Specification

### 12.1 Runtime

- Isolated execution using a proven runtime such as Deno isolates, WebAssembly, or hardened containers.
- No unrestricted host filesystem or process access.
- Configurable CPU, memory, wall-time, request-size, response-size, and outbound-network limits.
- Cold-start and concurrency behavior documented.
- Versioned deployments and rollback.

### 12.2 Developer Experience

- Local function serving.
- CLI deploy, list, inspect, logs, secrets, and delete commands.
- TypeScript-first support.
- Request/response HTTP API.
- Environment and project secrets.
- Database/Auth/Storage client access using scoped project credentials.

### 12.3 Security

- Secret encryption at rest.
- Secrets excluded from logs and build output.
- Dependency and bundle size limits.
- Egress policy and SSRF protection.
- Signed deployment artifacts.
- Per-function authorization configuration.
- Audit events for deploy, secret access, rollback, and deletion.

### 12.4 Observability

- Invocation count, status, latency, memory, CPU, and cold starts.
- Structured function logs with project/function/version context.
- Distributed trace propagation.
- Retention and redaction policies.

---

## 13. Studio Specification

Studio is an authenticated operational application, not a decorative dashboard.

Required modules:

- Project overview and health.
- SQL editor with saved queries, results, errors, cancellation, and explain plans.
- Table editor with schema-aware forms, filtering, sorting, pagination, and bulk operations.
- Schema browser for tables, columns, constraints, indexes, views, functions, policies, and relationships.
- Migration history and schema diff.
- Authentication users, sessions, providers, templates, and security settings.
- Storage bucket/object management.
- Realtime channel diagnostics.
- Function deployment, versions, secrets, and logs.
- API documentation generated from the active schema.
- Database connections and credential rotation.
- Logs, metrics, slow queries, and audit events.
- Backup/PITR and restore workflows.
- Project settings, domains, TLS, resource limits, and deletion controls.

Studio requirements:

- Responsive desktop and tablet operation.
- Keyboard-accessible workflows.
- No secrets rendered after initial creation unless explicitly recoverable.
- Destructive operations require clear confirmation and display exact scope.
- Long operations expose progress and can recover after browser refresh.
- All administrative actions use the same versioned admin APIs available to the CLI.

---

## 14. CLI and Local Development Specification

### 14.1 CLI

The CLI must support:

- Login and project selection.
- Initialize local project configuration.
- Start, stop, status, and reset local stack.
- Link and unlink a remote project.
- Generate, validate, diff, apply, repair, and list migrations.
- Generate database types for supported languages.
- Deploy and manage functions.
- Manage secrets.
- Inspect logs and health.
- Backup and restore commands.
- Stable JSON output for automation.
- Confirmation and explicit force flags for destructive actions.

### 14.2 Local Stack

- One-command startup using containers.
- Database, Auth, API, Realtime, Storage, Functions, Studio, mail catcher, and observability development services.
- Seed data and migrations.
- Persistent or disposable modes.
- Deterministic port configuration.
- Health checks and dependency ordering.
- Cross-platform support for Linux, macOS, and Windows through Docker.

### 14.3 Migrations

- Ordered versioned files.
- Checksums and immutable applied history.
- Advisory migration lock.
- Dry run and validation.
- Transactional application where supported.
- Repair workflow requiring explicit approval.
- Schema diff generation reviewed before application.
- CI mode that fails on drift.

Splitting migration SQL on semicolons is insufficient. A real SQL parser or database protocol capable of multi-statement migration execution is required.

---

## 15. Client SDK Specification

### 15.1 Initial SDKs

Priority order:

1. JavaScript/TypeScript.
2. Dart/Flutter.
3. Python.
4. Swift.
5. Kotlin.

### 15.2 SDK Modules

- Client creation and configuration.
- Auth sign-up, sign-in, session refresh, MFA, OAuth, and logout.
- REST query builder.
- GraphQL access or generated client integration.
- Realtime channels, presence, broadcast, and database changes.
- Storage uploads, downloads, signed URLs, and transforms.
- Function invocation.
- Typed error model.
- Retry and cancellation semantics.

### 15.3 SDK Quality

- Semantic versioning.
- Generated and hand-written compatibility tests.
- Browser, Node, mobile, and server runtime coverage as applicable.
- Tree-shakable modules for JavaScript.
- No service-role secret use in public-client helpers.
- Automatic token refresh with race control.
- Network offline/reconnect behavior documented.

---

## 16. Control Plane Specification

### 16.1 Organization and Project Model

- Organizations contain members and projects.
- Organization roles and project roles are separate from database roles.
- Projects have region, plan, version, status, resource allocation, and lifecycle state.
- Invitations, membership changes, and ownership transfers are audited.

### 16.2 Provisioning

- Asynchronous idempotent project creation.
- Database and service credential generation.
- Secret encryption.
- DNS and TLS provisioning.
- Migration to desired service versions.
- Health-based readiness transition.
- Failed-provisioning recovery and cleanup.

### 16.3 Project Lifecycle

- Pause and resume.
- Upgrade.
- Scale resources.
- Rotate credentials.
- Transfer organization.
- Schedule deletion with recovery window.
- Final secure deletion with auditable completion.

### 16.4 Resource Metering

- Database compute/storage.
- Egress.
- Realtime connections/messages.
- Storage bytes and operations.
- Function invocations and compute time.
- Auth active users where required.

Billing integration is not required for self-hosted mode but metering interfaces must be defined before managed GA.

---

## 17. Configuration and Secrets

- Typed configuration with defaults, file, environment, and CLI precedence.
- Startup validation with actionable errors.
- Secret values supplied through files, environment secret injection, or secret manager integration.
- No production default JWT or database secret.
- Key rotation without full outage.
- Separate public, anonymous, service, database, replication, and control-plane credentials.
- Configuration reload only for explicitly reloadable settings.
- Configuration schema versioning.
- Redacted diagnostic output.

---

## 18. Security Requirements

### 18.1 Secure Development

- Threat model for each public service.
- Security review required before external alpha.
- Dependency vulnerability scanning.
- Secret scanning.
- Static analysis and strict lint gates.
- Fuzzing for parsers, protocol framing, SQL, policy expressions, JWT handling, and file formats.
- Reproducible or traceable release builds.
- Signed release artifacts and container images.

### 18.2 Network Security

- TLS 1.2 minimum, TLS 1.3 preferred.
- Secure cipher configuration through maintained TLS libraries.
- Certificate rotation.
- HTTP security headers.
- Request and header limits.
- Read, write, idle, and total timeouts.
- Connection and concurrency limits.
- Protection against request smuggling and malformed protocol frames.

### 18.3 Tenant Isolation

- Every data-plane resource belongs to exactly one project.
- Project identity is derived from trusted routing/configuration, not user-controlled payload fields.
- Cross-project queries and object access are impossible through normal credentials.
- Backup, logs, metrics, cache, queues, and temporary files preserve project isolation.
- Automated adversarial tests attempt cross-tenant access through every public interface.

### 18.4 Audit Logging

Audit events include:

- User and administrator authentication changes.
- Role, grant, policy, and service-key changes.
- Project settings and secret changes.
- Backup and restore operations.
- Function deploys and secret changes.
- Storage bucket policy changes.
- Privileged database access.
- Project deletion and recovery.

Audit logs are append-oriented, access-controlled, exportable, and protected from ordinary project data mutation.

### 18.5 Compliance Readiness

GA architecture must support:

- Data retention configuration.
- User data export and deletion workflows.
- Regional data placement.
- Encryption in transit and at rest.
- Access reviews.
- Incident response evidence.
- Backup retention and deletion.

Formal certifications are separate business projects and are not implied by technical readiness.

---

## 19. Reliability, Backup, and Disaster Recovery

### 19.1 Backups

- Scheduled full/base backups.
- Continuous log/WAL archiving.
- Encryption in transit and at rest.
- Checksums and restore validation.
- Retention policies.
- Backup inventory and status.
- Independent storage failure domain.

### 19.2 Point-in-Time Recovery

- Restore to a selected timestamp or log position.
- Recovery preview and estimated data-loss window.
- Restore into a new project by default.
- Explicit destructive in-place restore workflow if supported.
- Coordinated recovery for database and object-storage metadata.

### 19.3 Disaster Recovery

- Documented RPO and RTO by service tier.
- Regular automated restore drills.
- Region-loss runbook.
- Credential rotation after incident.
- Dependency outage behavior.
- Status communication hooks.

---

## 20. Observability Specification

### 20.1 Metrics

Prometheus-compatible metrics must cover:

- Service health and build version.
- Connections, requests, subscriptions, and function invocations.
- Latency histograms and error rates.
- Active transactions, lock waits, deadlocks, and cancellations.
- Buffer/cache hit rate, disk I/O, WAL rate, checkpoint duration, and replication lag.
- Auth failures, token refreshes, MFA challenges, and rate-limit actions.
- REST/GraphQL query timing.
- Realtime lag, queue depth, dropped messages, and reconnects.
- Storage bytes, requests, failures, and multipart cleanup.
- Backup status and restore drill results.

High-cardinality labels such as user ID, raw SQL, object path, token, or unrestricted error text are forbidden.

### 20.2 Logs

- Structured JSON logs in production.
- Request, connection, transaction, project, component, and trace identifiers.
- Configurable levels and sampling.
- Sensitive-value redaction.
- Separate audit and application logs.
- Rotation and retention guidance.

### 20.3 Tracing

- OpenTelemetry-compatible trace propagation.
- Cross-service spans from API/Auth/Realtime/Storage/Functions to database operations.
- Sampling controls.
- No sensitive payload capture by default.

### 20.4 Health

- Startup probe.
- Liveness probe.
- Readiness probe.
- Dependency/degraded health detail on protected admin endpoints.
- Health endpoints must not report ready before migrations, recovery, and required dependencies complete.

---

## 21. Performance and Capacity Targets

Exact targets must be benchmarked on declared hardware. Initial production objectives are:

- No unbounded memory growth under supported request patterns.
- Stable p95 latency under documented concurrency limits.
- Database queries cancel within a bounded interval after timeout.
- Realtime slow consumers cannot exhaust server memory.
- Large sorts/aggregations spill or fail with a resource-limit error rather than crashing.
- Backup and checkpoint activity remains observable and bounded.
- Project quotas are enforced before resource exhaustion.

Before beta, publish benchmark profiles for:

- Point reads and indexed range reads.
- Inserts and updates under concurrency.
- Mixed transactional workloads.
- REST and GraphQL overhead.
- Realtime fan-out.
- Auth sign-in and refresh.
- Storage upload/download.
- Function cold and warm invocation.

No benchmark may be advertised without the dataset, hardware, configuration, duration, and percentile methodology.

---

## 22. Service-Level Objectives

Initial GA objectives for a managed single-region deployment:

- Data-plane API availability: 99.9% monthly.
- Database connection availability: 99.9% monthly.
- Auth availability: 99.9% monthly.
- Realtime connection establishment: 99.9% monthly.
- Backup success: 99.9% of scheduled backups.
- Restore drills: 100% completion on the defined schedule.
- Security token/key rotation: no unplanned data-plane outage.

RPO and RTO must be declared per deployment tier. Self-hosted deployments receive tooling and documentation, not a hosted SLO guarantee.

---

## 23. Testing Strategy

### 23.1 Test Layers

- Unit tests for pure logic and encoding.
- Property tests for storage, parser, planner, policy, and protocol invariants.
- Integration tests for each service with real dependencies.
- End-to-end tests across SDK, gateway, auth, database, realtime, storage, and functions.
- Differential SQL tests against PostgreSQL for claimed-compatible behavior.
- Compatibility tests with real clients and ORMs.
- Fuzz tests for all public parsers and binary formats.
- Fault-injection tests for I/O, disk full, network interruption, clock skew, process death, and dependency failure.
- Load, soak, and resource-exhaustion tests.
- Security and tenant-isolation tests.
- Backup/restore and disaster-recovery drills.

### 23.2 Mandatory CI Gates

- Formatting.
- Strict linting with reviewed exceptions.
- Unit and integration tests.
- Security and dependency scans.
- Documentation link and example validation.
- Container build and startup.
- Migration up/down or forward/restore validation as applicable.
- Cross-platform build matrix.
- No dirty generated artifacts after tests.

### 23.3 Release Candidate Gates

- Full regression suite.
- Upgrade from previous supported release.
- Backup and restore validation.
- Process-kill recovery.
- Cross-tenant adversarial suite.
- Load and soak test.
- Dependency license/security report.
- Signed artifacts.
- Runbooks reviewed against actual commands.

---

## 24. Documentation Requirements

Documentation shipped with each release must include:

- Architecture overview.
- Supported SQL and PostgreSQL compatibility matrix.
- API and SDK references.
- Authentication flows.
- RLS and policy examples.
- Realtime delivery guarantees.
- Storage authorization model.
- Function runtime limits.
- Configuration reference.
- Local development guide.
- Production deployment guide.
- Backup, restore, upgrade, rollback, and incident runbooks.
- Security model and secret rotation.
- Known limitations and deferred features.

Stale statements that contradict the code are release blockers.

---

## 25. Master Implementation Roadmap

Every phase begins with a requirements review and ends with evidence attached to its exit criteria. Time estimates assume an experienced team and are planning ranges, not commitments.

### Phase 0: Baseline Stabilization and Security Containment

**Objective:** Make the existing repository an honest, reproducible, private-development baseline.

**Work:**

- Remove unauthenticated administrator behavior.
- Disable or protect public TCP/HTTP access until authentication is real.
- Replace prototype password and JWT code with maintained libraries or temporarily disable Auth issuance.
- Prevent client-selected administrator roles.
- Eliminate SQL string construction from REST/Auth input.
- Repair Docker build version, entrypoint, bind addresses, health check, and non-root runtime.
- Integrate or remove misleading unused concurrency components.
- Make formatting and strict lint gates pass.
- Update README and gap analysis.
- Add CI for build, test, lint, format, and container startup.
- Record a database format version and current compatibility statement.

**Exit criteria:**

- No known unauthenticated admin path.
- No hand-rolled cryptographic authentication primitive remains exposed.
- Container builds and starts with persistent data and working health checks.
- All default tests pass in one CI run.
- Ignored scale tests have an explicit scheduled profile.
- Documentation matches reality.

**Estimated team duration:** 3-6 weeks.

### Phase 1: Production Service Foundation

**Objective:** Establish service lifecycle, configuration, networking, telemetry, and resource governance.

**Work:**

- Typed configuration and secret loading.
- Standard HTTP framework.
- TLS support or documented trusted reverse-proxy boundary.
- PostgreSQL wire foundation.
- Connection/session model and cancellation.
- Bounded worker/execution pools.
- Timeouts, request limits, connection limits, and graceful drain.
- Structured logs, metrics, traces, and health probes.
- Admin listener separated from public listeners.
- Stable error model and SQLSTATE mapping.

**Exit criteria:**

- Real clients connect securely.
- Malformed/slow/oversized clients cannot exhaust resources.
- Disconnects cleanly release sessions and transactions.
- Shutdown drains or rolls back within configured bounds.
- Operational telemetry is sufficient to diagnose failures.

**Estimated team duration:** 2-3 months.

### Phase 2: Database Kernel and PostgreSQL Compatibility

**Objective:** Deliver the relational and transactional foundation required by all platform services.

**Strategy A work:**

- Provision and configure PostgreSQL.
- Define supported versions and extensions.
- Establish migrations, roles, schemas, backups, pooling, and replication conventions.
- Build compatibility and lifecycle automation around PostgreSQL.

**Strategy B Native-only work:**

- MVCC and true concurrent readers/writers.
- WAL, checkpoints, redo recovery, vacuum/garbage collection.
- Cost-based planner and statistics.
- Complete type, constraint, schema, query, and index requirements.
- PostgreSQL extended protocol and client compatibility.
- Replication primitives and PITR log positions.

**Shared work:**

- Roles, grants, service roles, and complete RLS semantics.
- Migration locking and checksums.
- Query cancellation and resource limits.
- Differential/compatibility test suites.

**Exit criteria:**

- Claimed SQL semantics match compatibility tests.
- Concurrent transaction tests demonstrate isolation and constraint correctness.
- Crash and restart preserve committed data.
- RLS cannot be bypassed through any supported query path.
- Selected clients and ORMs pass documented workflows.

**Estimated team duration:**

- Strategy A: 2-4 months.
- Strategy B: 12-30+ months before comparable maturity.

### Phase 3: Production Auth and Authorization

**Objective:** Ship secure identity, session, provider, MFA, and administrative flows.

**Work:**

- Identity schema and tenant isolation.
- Argon2id credentials.
- JWT/JWKS and rotating refresh tokens.
- Email/password, magic links, OTP, and OAuth/OIDC.
- Verification, recovery, linking, and deletion.
- MFA.
- Rate limits and abuse controls.
- Auth hooks and provider abstractions.
- Admin APIs and Studio module.
- Database claim propagation and RLS integration.

**Exit criteria:**

- Threat model reviewed.
- Token rotation, revocation, reuse, expiry, and tamper tests pass.
- User enumeration and brute-force controls are verified.
- Public sign-up cannot produce privileged roles.
- All auth paths propagate immutable verified identity to the database.

**Estimated team duration:** 4-7 months.

### Phase 4: REST, GraphQL, and SDK Foundation

**Objective:** Provide typed, secure, schema-driven application APIs.

**Work:**

- Full REST query grammar and relationship embedding.
- Typed parameterized database execution.
- Function RPC.
- GraphQL schema generation and limits.
- OpenAPI/schema documentation.
- JavaScript/TypeScript SDK.
- Auth-aware request handling.
- CORS, CSRF, idempotency, caching, and quotas.
- API Studio/documentation screens.

**Exit criteria:**

- REST and GraphQL return the same authorized data as equivalent SQL.
- RLS adversarial tests cover every operation.
- Generated schemas update safely after migrations.
- SDK session refresh and concurrent request behavior are reliable.
- No route constructs SQL through untrusted string concatenation.

**Estimated team duration:** 4-6 months.

### Phase 5: Realtime Platform

**Objective:** Deliver authorized database changes, broadcast, and presence at scale.

**Work:**

- WAL/logical change consumer.
- Durable positions and retention.
- WebSocket protocol.
- RLS-aware filtering.
- Broadcast and presence.
- Backpressure and slow-consumer handling.
- Horizontal fan-out.
- SDK channel APIs.
- Realtime Studio diagnostics.

**Exit criteria:**

- Rollbacks never emit.
- Resume semantics match documentation.
- Unauthorized rows never reach subscribers.
- Load tests meet declared connection/fan-out targets.
- Node failure and reconnect behavior are tested.

**Estimated team duration:** 4-7 months.

### Phase 6: Object Storage

**Objective:** Add RLS-integrated S3-compatible file storage.

**Work:**

- Bucket/object metadata schema.
- S3 and local backend adapters.
- Upload/download/list/move/copy/delete APIs.
- Multipart uploads and signed URLs.
- RLS authorization.
- Checksums, cleanup, quotas, and lifecycle.
- Image transforms.
- SDK and Studio storage modules.

**Exit criteria:**

- Metadata and binary state remain consistent under failure.
- Cross-tenant object access tests pass.
- Signed URL scope and expiry tests pass.
- Large/resumable uploads survive interruption.
- Backup/restore procedures cover metadata and content.

**Estimated team duration:** 3-5 months.

### Phase 7: Functions Runtime

**Objective:** Provide safe deployable server-side functions.

**Work:**

- Runtime selection and isolation.
- Bundle/deployment service.
- Versioning and rollback.
- Secrets and environment management.
- Limits, metering, networking policy, and logs.
- CLI local serve and deploy.
- SDK invocation and Studio management.

**Exit criteria:**

- Sandbox escape review completed.
- Resource limits hold under hostile workloads.
- Secrets do not appear in logs or artifacts.
- Deploy/rollback is atomic and auditable.
- Function failures do not destabilize other tenants.

**Estimated team duration:** 4-7 months.

### Phase 8: Studio, CLI, and Developer Experience Completion

**Objective:** Make all shipped platform capabilities operable without internal knowledge.

**Work:**

- Complete Studio modules.
- Complete CLI workflows.
- Local development stack.
- Type generation.
- Migration diff/apply/repair.
- API documentation and examples.
- Onboarding and project diagnostics.
- Additional SDKs.

**Exit criteria:**

- A new developer can start locally, migrate, build an authenticated app, subscribe to changes, upload files, deploy a function, and inspect logs using documented workflows.
- Studio and CLI use public versioned admin APIs.
- Destructive operations are protected and audited.
- Cross-platform local development tests pass.

**Estimated team duration:** 4-8 months, overlapping Phases 3-7.

### Phase 9: HA, PITR, Operations, and Managed Control Plane

**Objective:** Reach managed production reliability and project lifecycle automation.

**Work:**

- Replicas, promotion, fencing, and failover runbooks.
- Continuous backups and PITR.
- Restore drills and disaster recovery.
- Organizations, members, projects, provisioning, domains, and TLS.
- Version rollout and rollback.
- Metering and quota enforcement.
- Fleet observability and incident tooling.
- Secure deletion and retention.

**Exit criteria:**

- Failover and restore exercises meet declared RPO/RTO.
- Project provisioning is idempotent and recoverable.
- Control-plane outage does not unnecessarily stop healthy data planes.
- Upgrades and rollbacks are tested across supported versions.
- Tenant isolation tests cover infrastructure and operational data.

**Estimated team duration:** 6-12 months.

### Phase 10: External Beta and GA Hardening

**Objective:** Convert a feature-complete beta into a supportable production release.

**Work:**

- External security assessment.
- Long-duration soak and chaos testing.
- Performance tuning and capacity models.
- Documentation and support runbooks.
- Deprecation/versioning policy.
- Incident response process.
- Signed releases and SBOMs.
- Final compatibility matrix.
- Beta migration and upgrade feedback.

**Exit criteria:**

- No unresolved critical/high security findings.
- SLO monitoring and alerting operational.
- Restore and failover drills pass.
- Upgrade from the previous supported release passes.
- Known limitations are published.
- On-call and incident ownership are defined.

**Estimated team duration:** 3-6 months after feature beta.

---

## 26. Dependency Order and Critical Path

The critical path is:

1. Security containment and reproducible baseline.
2. Database strategy decision.
3. Production database transactions, protocol, RLS, and operations.
4. Auth identity and trusted claim propagation.
5. REST/GraphQL and first SDK.
6. Realtime and Storage.
7. Functions.
8. HA/PITR and managed control plane.
9. External beta and GA hardening.

Studio, CLI, documentation, observability, testing, and security are continuous workstreams. They are not final polish phases.

Auth, API, Realtime, and Storage must not independently invent authorization. They must converge on the database identity and RLS contract.

---

## 27. Release Milestones

### Private Development Baseline

- Phase 0 complete.
- No public exposure.
- Reproducible local stack.

### Developer Preview

- Secure database service.
- PostgreSQL client access.
- Basic production Auth and REST.
- RLS end-to-end.
- JS/TS SDK.
- No availability promise.

### Private Alpha

- Realtime database changes.
- Storage MVP.
- Studio core workflows.
- Backups and manual restore.
- Selected external users under explicit limitations.

### Public Beta

- GraphQL, broadcast/presence, Functions MVP.
- Replicas and PITR.
- Published limits and compatibility matrix.
- Security review and load tests.
- Upgrade support.

### General Availability

- GA release gates pass.
- SLOs, support ownership, incident process, backups, restore drills, signed releases, security program, and documentation are operational.

---

## 28. Staffing and Delivery Reality

A full Supabase-class platform is not a single-feature database project. It requires simultaneous expertise in:

- Database storage, MVCC, query execution, and replication.
- Backend/API engineering.
- Identity and application security.
- Realtime distributed systems.
- Object storage.
- Runtime sandboxing.
- Frontend product engineering.
- Infrastructure/SRE.
- SDK and developer tooling.
- Security testing and incident response.

Indicative minimum experienced team for serious parallel delivery:

- 3-5 database/backend engineers.
- 2 Auth/security engineers.
- 2 API/Realtime engineers.
- 2 Storage/Functions engineers.
- 2 frontend/Studio engineers.
- 1-2 SDK/CLI engineers.
- 2 infrastructure/SRE engineers.
- Dedicated security and QA ownership, whether internal or contracted.

With Strategy A and a focused experienced team, a credible beta remains a multi-quarter program. With Strategy B, database-kernel maturity alone can require multiple years. These estimates must not be converted into promises before architecture spikes and measured velocity.

---

## 29. Major Risks

| Risk | Impact | Required mitigation |
| --- | --- | --- |
| Hand-built database scope overwhelms platform work | Critical schedule risk | Decide Strategy A vs B before Phase 2; keep compatibility matrix honest |
| Security prototype reaches public deployment | Critical compromise risk | Phase 0 containment; fail closed; external review before public beta |
| RLS differs across SQL/API/Realtime/Storage | Cross-tenant data exposure | One identity contract and central database enforcement |
| Custom protocol blocks ecosystem adoption | High product risk | PostgreSQL wire and real-client compatibility tests |
| Unbounded queues or queries exhaust resources | Availability failure | Budgets, timeouts, backpressure, quotas, load tests |
| Backups exist but restores fail | Permanent data loss | Automated restore drills and measured RPO/RTO |
| Documentation claims exceed implementation | Unsafe operator behavior | Documentation gates and executable examples |
| Control plane gains excessive project access | Large blast radius | Separate trust domains, scoped credentials, audit, encryption |
| Function runtime escape | Cross-tenant compromise | Proven isolation runtime, hard limits, security assessment |
| Migration/schema drift corrupts environments | Deployment failure | Checksums, locks, validation, drift detection, tested rollback/restore |

---

## 30. Definition of Done

A feature is done only when:

- Requirements and failure semantics are documented.
- Threat model impact is reviewed.
- Implementation uses approved dependencies and patterns.
- Unit, integration, adversarial, and end-to-end tests pass as applicable.
- Resource limits and timeouts are defined.
- Metrics, logs, and traces are sufficient to operate it.
- Upgrade, rollback, backup, and compatibility impact are understood.
- Public API and SDK behavior is documented.
- Containers and local workflows exercise it.
- No known critical or high-severity issue remains unresolved.
- The feature is included in release notes and the compatibility matrix.

Passing one happy-path integration test is not sufficient.

---

## 31. Explicit Non-Goals for Initial GA

The following are not required for the first GA unless later approved:

- Exact compatibility with every Supabase API extension or internal service.
- Every PostgreSQL extension.
- Multi-primary writes.
- Transparent active-active cross-region databases.
- Enterprise SAML/SCIM.
- Formal compliance certification.
- Built-in billing and payment collection for self-hosted mode.
- Kubernetes operator.
- Marketplace for third-party integrations.
- AI-generated SQL or application scaffolding.

Deferring these items does not permit weakening tenant isolation, authentication, durability, backup, or release safety.

---

## 32. Execution Authorization Protocol

This document is a plan only.

No code, dependencies, database formats, containers, schemas, or public APIs should be changed under this plan until the owner sends an explicit instruction such as:

> Execute Phase 0 from `supabase-level-spec.md`.

When a phase is authorized, work must begin by:

1. Re-reading this specification.
2. Auditing the current repository because it may have changed.
3. Producing a bounded phase task list mapped to the exit criteria.
4. Identifying destructive or compatibility-changing actions before performing them.
5. Implementing and verifying only the authorized phase or work package.

Until that authorization is given, this file is the final planning artifact and no implementation is authorized.
