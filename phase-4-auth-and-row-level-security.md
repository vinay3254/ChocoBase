# Phase 4: Auth & Row-Level Security Implementation Plan

## Objective

Add production-oriented user authentication, signed sessions, tenant-aware execution context, and row-level security enforced inside the database engine.

## Prerequisites

- Phase 2 connection sessions and Phase 3 expression/planner infrastructure are stable.
- Security-sensitive dependencies and threat model are reviewed before implementation.

## Documentation Gate

Create Phase 4 `requirements.md`, `design.md`, and `tasks.md`. The design must include a threat model, credential lifecycle, JWT key management, policy semantics, bypass rules, failure behavior, and a complete trace of authenticated claims from the network handshake to query execution.

## Identity Flow

1. The network layer authenticates credentials or verifies a JWT.
2. It constructs an immutable `SessionClaims` object containing user ID, role, tenant ID, expiry, and approved custom claims.
3. The connection session attaches claims to every execution request.
4. The planner/executor receives an `ExecutionContext`; SQL functions such as `auth.uid()` read only from this context.
5. The policy rewriter binds policy expressions to the context and table row.
6. All scans and mutations enforce applicable policies before returning or changing data.

## Implementation Work

### User and Credential Model

- Add internal auth schema for users, credential versions, sessions, and revocation metadata.
- Implement registration with normalized identifiers and uniqueness guarantees.
- Hash passwords with Argon2id using versioned parameters and per-password salts.
- Never log credentials, hashes, raw tokens, or sensitive claims.
- Add password verification, hash-upgrade-on-login, disabled-user checks, and rate-limit hooks.

### JWT Sessions

- Issue short-lived access tokens with issuer, audience, subject, issued-at, expiry, token ID, role, and tenant claims.
- Verify algorithm, signature, issuer, audience, expiry, and not-before fields.
- Support key rotation through key IDs and overlapping verification keys.
- Define refresh-token storage and revocation, or explicitly defer refresh tokens.

### SQL and Policies

- Add `CREATE POLICY`, `ALTER POLICY`, `DROP POLICY`, and table-level RLS enable/disable syntax.
- Support command scopes for `SELECT`, `INSERT`, `UPDATE`, `DELETE`, and `ALL`.
- Support `USING` filters and `WITH CHECK` validation.
- Add `auth.uid()` and approved claim-access functions to the expression engine.
- Store parsed/validated policy metadata in the catalog.

### Enforcement

- Apply policies in a central planner/executor boundary that cannot be bypassed by sequential scans, indexes, joins, subqueries, REST APIs, or realtime consumers.
- Use default-deny behavior when RLS is enabled and no policy grants access.
- Define owner/admin bypass narrowly and audit every bypass.
- Prevent policy recursion, timing leaks where practical, and claim spoofing through SQL variables.

## Testing

- Password hashing and verification tests using real Argon2id operations.
- JWT expiry, wrong-key, wrong-audience, key-rotation, tampering, and revocation tests.
- End-to-end network authentication tests.
- Multi-tenant isolation tests proving users cannot read or mutate another tenant's rows through scans, indexes, joins, aggregates, or errors.
- Concurrent policy-change and transaction tests.
- Fuzz/property tests for policy expression handling.
- Full earlier-phase regression suites with RLS both disabled and enabled.

## Acceptance Criteria

- Plaintext passwords are never stored.
- Every authenticated query carries immutable verified claims.
- `auth.uid()` has one documented source of truth from network session to executor.
- RLS is enforced consistently across all access paths and defaults to deny.
- Security failures are audited without exposing secrets.

## High-Risk Review Points

- JWT signing algorithm and key storage.
- Refresh-token strategy.
- Administrative bypass semantics.
- Policy behavior for foreign keys, cascades, and privileged maintenance tasks.

