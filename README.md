# ChocoBase

A SQL database engine built from scratch in Rust: a page-based storage layer,
a B+Tree used for both table storage and secondary indexes, a hand-written
SQL parser, a rule-based query planner, and a Volcano-model executor — driven
from an interactive REPL.

This is a learning project. It prioritizes every layer being readable and
testable in isolation over raw performance or SQL completeness.

## Usage

```bash
cargo run -- mydb.db
```

At the `dbengine>` prompt, type SQL statements terminated by `;`:

```sql
CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, active BOOLEAN);
INSERT INTO users (id, name, active) VALUES (1, 'Ada', TRUE), (2, 'Bea', FALSE);
SELECT name FROM users WHERE active = TRUE;
CREATE INDEX idx_name ON users (name);
UPDATE users SET active = TRUE WHERE id = 2;
DELETE FROM users WHERE id = 1;
```

Meta-commands (no trailing `;`):

| Command | Effect |
| --- | --- |
| `.tables` | list tables |
| `.schema <table>` | show the table's `CREATE TABLE` text |
| `.indexes <table>` | list indexes on a table |
| `.btree <table>` | dump the table's B+Tree structure |
| `.stats` | page count, freelist length, cache size, pages read by the last statement |
| `.exit` | quit |

## What's supported

- `CREATE TABLE` / `DROP TABLE` — one `INTEGER`, `TEXT`, or `BOOLEAN` column
  declared `PRIMARY KEY` is required; there is no implicit rowid.
- `CREATE INDEX` / `DROP INDEX` on a single `NOT NULL` column.
- `INSERT`, `SELECT` (projection, `WHERE`, `ORDER BY`, `LIMIT`), `UPDATE`, `DELETE`.
- `WHERE` supports `=`, `<>`/`!=`, `<`, `<=`, `>`, `>=`, `AND`, `OR`, `IS [NOT] NULL`,
  with standard precedence (`OR` loosest, then `AND`, then comparisons).

## What's deliberately not supported

These are not bugs — they are scope cuts made explicitly in the design spec
(`docs/superpowers/specs/2026-07-26-database-engine-design.md`) to keep every
layer buildable and readable from scratch:

- **No joins, aggregates, `GROUP BY`, or subqueries.**
- **No `FLOAT` type.**
- **`NULL` compares as false, not as SQL's three-valued unknown.** Every
  comparison operator (`=`, `<`, etc.) involving a `NULL` operand evaluates to
  `false`, never to "unknown." `IS NULL` / `IS NOT NULL` are the only way to
  test for it. Indexed columns and primary keys must be `NOT NULL`.
- **No transactions, no rollback.** Every statement autocommits: on success,
  its dirty pages are flushed and `fsync`ed. There is no journal and no undo.
  A crash partway through a multi-page B+Tree split, or a multi-row `INSERT`
  that fails partway through, can leave the file in a partially-written state.
  `UPDATE` is implemented as delete-then-insert on the same row: if the new
  values would violate a constraint (most notably, setting the primary key to
  a value another row already uses), the delete has already mutated the live
  page before the insert's check fails, so the original row is gone with
  nothing replacing it rather than left unchanged — the same no-rollback
  limitation, just a sharper way to hit it. The natural next step — recorded
  but not built — is a rollback journal: copy each page's original bytes to a
  side file and `fsync` it before modifying the page in place, then delete the
  journal on commit or replay it on reopen after a crash.
- **A single row must fit in one 4KB page.** There is no overflow-page
  chaining for oversized rows; inserting one returns a `RowTooLarge` error.
- **Single process, single connection.** No concurrency, no locking, no
  client/server protocol.
- **`ORDER BY` sorts entirely in memory.** There is no external merge sort,
  so sorting a result set larger than available memory will not work.
- **The query planner is rule-based, not cost-based.** It has exactly three
  access-path rules, tried in order: primary-key equality (`TableSeek`),
  then indexed-column equality (`IndexSeek`), then sequential scan
  (`SeqScan`) — each falling through to the next only when the WHERE clause
  doesn't match. A top-level `OR` anywhere in the predicate disables both
  seek optimizations for that query.

## Architecture

```
REPL / library API   (src/repl.rs, src/engine.rs)
Executor              (src/exec/*  — Volcano-style pull iterators)
Planner                (src/plan/* — access-path selection, WHERE evaluation)
Parser                  (src/sql/*  — hand-written lexer + recursive-descent parser)
Catalog                  (src/catalog.rs — schema stored in the database itself)
B+Tree                    (src/btree/* — shared by table storage and indexes)
Pager                      (src/storage/* — 4KB pages, LRU cache, freelist, fsync)
```

Keys throughout the B+Tree layer are order-preserving byte strings — an
`INTEGER` key is its big-endian, sign-flipped 8-byte encoding; a `TEXT` key
is its UTF-8 bytes plus a `0x00` terminator — so plain byte comparison *is*
SQL ordering, and one B+Tree implementation serves both table storage
(keyed by primary key, row as payload) and secondary indexes (keyed by
indexed value + primary key, empty payload).

## Testing

```bash
cargo test               # unit, integration, and property tests
cargo test -- --ignored  # the 100k-row scale test (Task 43); slow, skipped by default
```
