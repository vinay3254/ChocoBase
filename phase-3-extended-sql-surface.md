# Phase 3: Extended SQL Surface Implementation Plan

## Objective

Expand ChocoBase from a basic transactional SQL engine into a practical relational engine with joins, aggregation, constraints, schema evolution, and richer value types.

## Prerequisites

- Phase 2 server and wire-protocol integration are stable.
- Parser, planner, executor, catalog, row encoding, and protocol type mapping have regression coverage.

## Documentation Gate

Create Phase 3 `requirements.md`, `design.md`, and `tasks.md` before coding. The design must define SQL semantics, null behavior, type coercion, constraint timing, schema-version compatibility, and the exact feature subset committed for the phase.

## Delivery Order

1. Extend the type system and expression evaluator first because constraints, aggregation, joins, and wire output depend on it.
2. Extend catalog metadata and row encoding with explicit schema versions.
3. Add constraints and defaults before schema evolution so `ALTER TABLE` can reuse the same validation rules.
4. Add joins and aggregation after expressions and planner metadata are stable.
5. Add protocol metadata and client compatibility tests last.

## Implementation Work

### Types

- Add `FLOAT`, fixed-precision `NUMERIC`, `DATE`, `TIMESTAMP`, `JSON`/`JSONB`, and `UUID` representations.
- Define parsing, comparison, ordering, serialization, index eligibility, null behavior, and PostgreSQL wire type mappings.
- Use established crates for decimal, date/time, JSON, and UUID semantics.
- Add explicit coercion rules and reject lossy implicit conversions.

### Constraints and Defaults

- Add `UNIQUE`, `FOREIGN KEY`, and `DEFAULT` catalog metadata.
- Enforce unique constraints with backing indexes.
- Enforce foreign keys for insert, update, and delete, initially with `NO ACTION`/`RESTRICT` semantics.
- Define transaction visibility and lock ordering for referenced rows to avoid races and deadlocks.
- Evaluate deterministic defaults during insertion; document treatment of time-based defaults.

### ALTER TABLE

- Implement `ADD COLUMN` and `DROP COLUMN` with schema-versioned row decoding.
- Define behavior for adding `NOT NULL` columns, defaults, indexes, foreign keys, and existing rows.
- Use transactional catalog changes and crash-safe table rewrites where required.
- Preserve backward compatibility or introduce an explicit database format version and migration path.

### Joins

- Extend the AST and name resolver for aliases and qualified columns.
- Implement inner and left joins with nested-loop execution as the required baseline.
- Add hash join for equality predicates if memory accounting and spill behavior can be implemented safely.
- Add planner selection rules and explain output where available.

### Aggregation

- Implement `COUNT`, `SUM`, `AVG`, `MIN`, and `MAX`.
- Add `GROUP BY` and `HAVING` with correct null and empty-input semantics.
- Add hash aggregation with bounded memory; document spill-to-disk as shipped or deferred.

## Testing

- Parser, type round-trip, coercion, and property tests.
- Cross-product tests for nulls, numeric boundaries, timestamps, malformed JSON, and UUID values.
- Real transaction races for unique and foreign-key enforcement.
- Process-kill tests during table rewrite and catalog mutation.
- Differential query tests comparing supported SQL results with PostgreSQL for joins and aggregates.
- Client-server tests verifying protocol type metadata and values.
- Full Phase 1 and Phase 2 regression suites.

## Acceptance Criteria

- The phase report explicitly marks every requested feature as shipped, partially shipped, or deferred.
- Constraints remain correct under concurrent transactions.
- Schema changes are atomic and recoverable.
- Supported joins and aggregates match documented SQL semantics.
- Rich types survive insert, query, index use, server transport, close, and reopen.

## Likely Deferrals

- Right/full/cross joins beyond required forms.
- Cascading foreign-key actions and deferrable constraints.
- Arbitrary-precision numeric math beyond selected precision limits.
- Hash join or aggregation spill if safe memory accounting is not ready.
- Complex `ALTER COLUMN`, rename, and type conversion operations.

