# Design: A SQL Database Engine From Scratch

**Date:** 2026-07-26
**Status:** Approved
**Language:** Rust

## Purpose

Build a single-file, single-process SQL database engine in Rust, from scratch, as a
learning artifact. Every layer that a real database has — pager, B+Tree, catalog,
parser, planner, executor — is implemented directly and is readable and testable on
its own.

The goal is understanding, not throughput. Where a simplification costs clarity, we
pay for the clarity. Where a simplification costs only performance, we take it.

## Scope

### In scope

- Single file on disk, 4KB pages, explicit page cache and freelist
- B+Tree used for both table storage and secondary indexes
- Hand-written SQL lexer and recursive-descent parser (no parser library)
- Rule-based query planner that chooses between index seek and sequential scan
- Volcano-model executor
- SQL: `CREATE TABLE`, `DROP TABLE`, `CREATE INDEX`, `DROP INDEX`, `INSERT`,
  `SELECT` (projection, `WHERE`, `ORDER BY`, `LIMIT`), `UPDATE`, `DELETE`
- Types: `INTEGER`, `TEXT`, `BOOLEAN`; integer primary key
- Durability: flush dirty pages and `fsync` at statement commit
- Interactive REPL over a database file

### Explicitly out of scope

These are named so that no one wonders whether they were forgotten:

- Joins, aggregates, `GROUP BY`, subqueries
- `FLOAT` and three-valued `NULL` logic (columns are nullable; comparisons against
  `NULL` yield false rather than unknown — see Type System)
- Transactions with rollback, write-ahead logging, crash recovery
- Concurrency of any kind — one process, one connection
- Client/server protocol
- Overflow pages for large rows
- Query optimization based on statistics or cost estimates

## Architecture

Six layers. Each depends only on the layer below it, and each is testable without
the layer above it.

```
REPL / library API
Executor        Volcano iterators: scan, filter, project, sort, limit, mutate
Planner         rule-based: index seek or sequential scan
Parser          lexer -> AST, recursive descent
Catalog         schema stored in the database itself
B+Tree          byte-string keys -> payloads, cursor with descent path
Pager           4KB pages, cache, freelist, fsync
```

The layering rule is a hard constraint, not a preference. The B+Tree must be
exercisable with no SQL involved; the pager must be exercisable with no B+Tree
involved. If a test for a lower layer needs to reach through an upper layer, the
boundary is wrong.

### Module layout

```
src/
  main.rs              REPL binary entry point
  lib.rs               public library API
  repl.rs              prompt loop, meta-commands, result formatting

  storage/
    pager.rs           page cache, allocation, freelist, fsync
    page.rs            raw 4KB page, slotted-page primitives
    header.rs          file header (page 0) read/write

  btree/
    node.rs            leaf and internal node encoding over a slotted page
    cursor.rs          seek, next, insert, delete; carries descent path
    balance.rs         split and merge/redistribute
    check.rs           check_invariants() tree walker

  types/
    value.rs           Value enum, order-preserving key encoding
    row.rs             row serialize/deserialize against a schema
    schema.rs          Column, ColumnType, TableSchema, IndexSchema

  catalog/
    mod.rs             schema persistence, lookup, create/drop

  sql/
    token.rs           token kinds
    lexer.rs           hand-written tokenizer
    ast.rs             statement and expression AST
    parser.rs          recursive descent + precedence climbing

  plan/
    planner.rs         predicate analysis, access-path selection
    plan.rs            plan node tree
    expr.rs            WHERE expression evaluation against a row

  exec/
    mod.rs             Operator trait
    scan.rs            SeqScan, TableSeek, IndexSeek
    filter.rs          Filter
    project.rs         Project
    sort.rs            Sort (in memory)
    limit.rs           Limit
    mutate.rs          Insert, Update, Delete

  error.rs             error enums per layer
```

## File format

One file. Page size 4096 bytes, fixed. Page 0 is the header and holds no user data.

### Header (page 0)

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 8 | magic `MINIDB\0\x01` |
| 8 | 2 | page_size (u16, always 4096) |
| 10 | 4 | page_count (u32) |
| 14 | 4 | freelist_head (u32, 0 = empty) |
| 18 | 4 | catalog_root (u32) |

Remaining header bytes are zero and reserved.

The magic is verified on open. A mismatch is a hard `NotADatabase` error, never a
best-effort recovery attempt.

### Page types

`1` internal node, `2` leaf node, `3` free.

Freed pages are threaded onto a singly-linked freelist through `freelist_head`; the
first 4 bytes of a free page point to the next free page. `allocate_page` pops from
the freelist when non-empty, otherwise extends the file and increments `page_count`.

## Pager

```rust
pub struct Pager {
    file: File,
    cache: LruCache<u32, Page>,
    dirty: HashSet<u32>,
    header: Header,
}
```

Responsibilities, and nothing else:

- `get_page(n)` / `get_page_mut(n)` — read through the cache, marking dirty on
  mutable access
- `allocate_page()` / `free_page(n)` — freelist management
- `flush()` — write all dirty pages, then `file.sync_all()`

The cache is bounded with LRU eviction. Only clean pages may be evicted; if the LRU
victim is dirty it is written before eviction. A bounded cache is deliberate — an
unbounded cache would quietly turn this into an in-memory database and hide the
cost model that makes B+Trees worth having.

The pager knows nothing about B+Trees, rows, or SQL.

## B+Tree

A B+Tree, not a plain B-Tree: internal nodes hold separator keys and child pointers
only, all payloads live in leaves, and leaves are chained through `next_leaf` so a
range scan is a linear walk after one descent.

### Node layout

Every node is a **slotted page**. The slot directory grows forward from the node
header; cells grow backward from the end of the page; free space sits in the middle.

```
+--------------------------------------------------+
| node header | slot directory ->    free    <- cells |
+--------------------------------------------------+
```

Node header:

| Field | Size | Notes |
| --- | --- | --- |
| page_type | 1 | 1 = internal, 2 = leaf |
| flags | 1 | reserved |
| num_cells | 2 | number of slots in use |
| free_start | 2 | end of slot directory |
| free_end | 2 | start of cell area |
| link | 4 | internal: rightmost_child; leaf: next_leaf |

The slot directory is an array of `u16` cell offsets held **in key order**. Sort
order therefore lives in the directory, so inserting into the middle of a node
shifts `u16` entries rather than relocating cell bytes.

Cells:

- **Leaf cell:** `key_len u16 | key bytes | payload_len u16 | payload bytes`
- **Internal cell:** `key_len u16 | key bytes | left_child u32`

Space reclaimed by deletes is tracked only as the gap between `free_start` and
`free_end`. Fragmented cell space is recovered by compacting the node in place when
an insert does not fit but total free space would allow it.

### No parent pointers

The cursor carries its descent path as an explicit stack of `(page_no, slot_index)`.
Splits and merges walk back up that stack. This removes an entire class of
parent-pointer maintenance bugs, and it costs nothing because every mutation
already arrived through a descent.

### Keys are order-preserving byte strings

This is the decision that lets one B+Tree implementation serve every purpose.

Keys are `Vec<u8>`, encoded so that **bytewise comparison is SQL ordering**:

- `INTEGER` — big-endian `i64` with the sign bit flipped, 8 bytes. Flipping the
  sign bit maps `i64::MIN..=i64::MAX` onto `u64` order, so negatives sort before
  positives.
- `TEXT` — UTF-8 bytes followed by a `0x00` terminator. Text values may not contain
  an interior NUL; this is validated at insert and rejected as `InvalidValue`.
- `BOOLEAN` — one byte, `0` or `1`.

Composite keys are the concatenation of the encoded parts. Because each encoding is
self-terminating or fixed-width, concatenation preserves lexicographic order.

Consequently `memcmp` *is* the comparator, and the node code never needs to know
what a key means.

### Two uses, one implementation

- **Table B+Tree** — key is the encoded integer primary key; payload is the whole
  serialized row.
- **Index B+Tree** — key is the encoded indexed value **with the encoded primary key
  appended**; payload is empty. Appending the primary key makes every index key
  unique, which lets non-unique indexes use exactly the same insert path. An index
  lookup for value `v` is a range scan over the prefix `encode(v)`, yielding primary
  keys, each of which is then fetched from the table B+Tree.

### Deliberate limitation: one row, one page

A row must fit within a single page. Rows that do not are rejected with
`RowTooLarge`. Overflow-page chaining is the single most intricate piece of B-Tree
engineering and teaches proportionally little, so it is cut. The limit is a
documented, enforced, clearly-reported error — never silent truncation.

Concretely: a node must be able to hold at least 3 cells, so the maximum payload is
bounded well below 4096 bytes. The exact bound is computed from the page size and
header sizes rather than hardcoded.

### Rebalancing

On insert, a full node splits at its midpoint and pushes a separator key up the
cursor's path stack, splitting ancestors as needed; a split at the root creates a
new root and increases tree height by one.

On delete, a node that falls below half capacity borrows from a sibling when the
sibling can spare a cell, and merges with it otherwise, removing the separator from
the parent and recursing upward. A root left with a single child is replaced by that
child, decreasing height.

## Type system

Three types: `INTEGER` (`i64`), `TEXT` (`String`), `BOOLEAN` (`bool`). Columns may be
declared `NULL` or `NOT NULL`.

`NULL` is representable and storable, but comparison semantics are deliberately
simplified: any comparison involving `NULL` evaluates to false rather than to SQL's
three-valued unknown. `IS NULL` and `IS NOT NULL` are the supported way to test for
null. This divergence from the SQL standard is intentional, documented in the
README, and noted as a natural follow-up.

Indexed columns must be `NOT NULL` in this version; `CREATE INDEX` on a nullable
column is rejected with a clear error rather than silently producing an index that
cannot represent the null rows.

### Row encoding

A row is serialized against its table schema as:

```
null_bitmap (ceil(num_columns / 8) bytes)
then, for each non-null column in schema order:
  INTEGER  8 bytes, little-endian i64
  BOOLEAN  1 byte
  TEXT     u16 length + UTF-8 bytes
```

Decoding walks the schema in the same order, so the encoding carries no field tags.
Schema changes beyond `CREATE`/`DROP TABLE` are out of scope, so there is no version
tag on rows.

Note that the row encoding is *not* the key encoding. Rows use little-endian for
speed of access; keys use the order-preserving encoding above. Keeping these
separate is intentional — conflating them is a common source of subtle ordering bugs.

## Catalog

The catalog is itself a B+Tree, rooted at the page recorded in `catalog_root` in the
file header. This removes any bootstrapping problem: the header is at a fixed
location and points to the catalog, and the catalog points to everything else.

- Key — the object name, encoded as `TEXT`
- Payload — a serialized catalog record:
  - kind (`table` or `index`)
  - name
  - root page number
  - for a table: the column list (name, type, nullable, is_primary_key)
  - for an index: the table name and the indexed column name

`CREATE TABLE` allocates a root page for a new empty leaf and inserts a catalog
record. `DROP TABLE` walks the table's pages onto the freelist, drops dependent
index records and their pages, and removes the catalog record.

The catalog is loaded into an in-memory map on open and kept in sync on every DDL
statement, so planning does not touch disk.

## Parser

Hand-written. Using a parser library here would skip the exercise.

### Lexer

Produces a token stream: keywords (case-insensitive), identifiers, integer literals,
single-quoted string literals with `''` escaping, operators
(`= <> != < <= > >= ( ) , * ;`), and end-of-input. Every token carries its byte
offset in the source.

### Parser

Recursive descent, one function per grammar production. `WHERE` expressions are
parsed by **precedence climbing** with this precedence, loosest first:

```
OR
AND
NOT
= <> != < <= > >=   IS NULL / IS NOT NULL
parentheses, literals, column references
```

Supported statements:

```sql
CREATE TABLE name (col TYPE [NOT NULL] [PRIMARY KEY], ...)
DROP TABLE name
CREATE INDEX name ON table (column)
DROP INDEX name
INSERT INTO table [(cols...)] VALUES (...), (...)
SELECT (* | column_list) FROM table [WHERE expr] [ORDER BY col [ASC|DESC]] [LIMIT n]
UPDATE table SET col = value, ... [WHERE expr]
DELETE FROM table [WHERE expr]
```

Parse errors carry the byte offset of the offending token so the REPL can print the
source line with a caret underneath.

## Planner

Rule-based, no statistics, no cost model. Four rules applied in priority order:

1. If `WHERE` contains an equality or range predicate on the **primary key**, emit
   `TableSeek` — a direct descent of the table B+Tree.
2. Otherwise, if it contains an equality or range predicate on an **indexed
   column**, emit `IndexSeek` — descend the index B+Tree, then fetch each row from
   the table B+Tree by primary key.
3. Otherwise emit `SeqScan` — descend to the leftmost leaf and walk the leaf chain.
4. Any predicate not satisfied by the chosen access path becomes a `Filter` above it.

Only conjunctions are analyzable for access-path selection. A top-level `OR` falls
back to `SeqScan` plus `Filter`, which is correct if unexciting.

The plan is then wrapped, in order, by `Project`, `Sort`, and `Limit` as the query
requires.

Because the planner is a distinct layer producing an explicit plan tree, `EXPLAIN`
is nearly free to add later and is a natural extension.

## Executor

The Volcano iterator model:

```rust
pub trait Operator {
    fn next(&mut self) -> Result<Option<Row>>;
}
```

Operators nest, and rows are pulled one at a time from the top. Every operator is
small enough to read in one sitting.

`Sort` is the one blocking operator and sorts in memory — it buffers its entire
input. This is an honest limitation for a learning engine, documented in the README,
with external merge sort noted as the natural follow-up.

Mutating statements (`Insert`, `Update`, `Delete`) are operators that consume rows
and report a count. `Update` and `Delete` **collect the full set of affected primary
keys before mutating**, rather than mutating during iteration. Mutating a B+Tree
while a cursor walks it invalidates the cursor's descent path; collecting first
avoids the problem outright and is the clearer design at this scale.

Index maintenance is synchronous: `Insert` adds an entry to every index on the
table, `Delete` removes them, and `Update` removes and re-adds entries for indexed
columns whose value changed.

## Durability

Autocommit, one statement at a time: on success, the pager writes all dirty pages
and calls `sync_all()`.

**There is no rollback.** This is a known, reachable limitation, and stating it
plainly is part of the point of the project:

- A crash partway through flushing a multi-page split can leave the file structurally
  inconsistent.
- A statement that fails midway — for example, a multi-row `INSERT` whose fourth row
  violates a constraint — may leave earlier rows written.

The README must say this explicitly. The natural next chapter is a rollback journal:
copy each original page to a side file and `fsync` it before modifying the page in
place, then delete the journal on commit and replay it on open if present. That is
roughly 200 lines and would make the engine genuinely crash-safe. It is deliberately
deferred so that the need for it is felt before it is built.

## Error handling

One error enum per layer, composed with `thiserror`:

- `StorageError` — I/O, `NotADatabase`, `CorruptPage`, `PageOutOfRange`
- `BTreeError` — `RowTooLarge`, `DuplicateKey`, `KeyNotFound`
- `ParseError` — message plus byte offset
- `PlanError` — `NoSuchTable`, `NoSuchColumn`, `NoSuchIndex`, `TypeMismatch`
- `ExecError` — `NotNullViolation`, `DuplicatePrimaryKey`, `InvalidValue`

Corruption is detected rather than assumed away: magic and page-type are verified on
every structural read, and `check_invariants()` walks a whole tree verifying key
ordering within and across nodes, fill factors, and that every leaf sits at the same
depth. It runs in tests and behind a debug flag, never on the hot path.

## Testing

The B+Tree is where correctness is won or lost, so testing is designed around it
rather than added after.

**Property tests** (`proptest`):

- *Key ordering* — for random `Value` pairs, `encode(a) < encode(b)` if and only if
  `a < b` under SQL ordering. This single property is what makes the byte-string key
  design safe; everything else depends on it.
- *B+Tree round trip* — insert N random keys, assert an in-order scan returns exactly
  those keys in sorted order; delete a random subset, assert again. Run
  `check_invariants()` after every mutation.
- *Row round trip* — encode then decode any row against its schema yields the
  original, including nulls.

**Unit tests** — slot directory insert/remove/compact, page allocation and freelist
reuse, lexer token streams, parser AST shapes, expression evaluation.

**Integration tests** — `.sql` script files paired with expected output, executed
end-to-end against a temporary database file.

**Persistence tests** — write data, drop the engine entirely, reopen the file, and
verify contents. This is the test that proves it is a database and not a data
structure.

**Index equivalence test** — for a set of queries, assert that the result with an
index present is identical to the result without one. This catches planner bugs that
correctness tests on a single path would miss.

## REPL

```
$ dbengine mydb.db
```

Statements are terminated by `;` and may span lines. Meta-commands:

| Command | Effect |
| --- | --- |
| `.tables` | list tables |
| `.schema [table]` | show `CREATE TABLE` text |
| `.indexes [table]` | list indexes |
| `.btree <table>` | dump the B+Tree structure — page numbers, heights, keys per node, fill |
| `.stats` | page count, freelist length, cache hit rate, and pages read by the last statement |
| `.exit` | flush and quit |

`.btree` is not a debugging afterthought; it is a primary learning feature. Watching
the tree change shape across inserts is the clearest possible demonstration of what
a B+Tree does.

## Dependencies

Runtime: `thiserror`, `rustyline` (REPL line editing and history).
Dev: `proptest`, `tempfile`.

Deliberately **not** used: `sqlparser` (parsing is the exercise), `serde` (the file
format is hand-rolled on purpose), any storage crate.

## Build order

Bottom-up. Each phase is fully green — tests passing — before the next begins.

1. Page primitives and file header
2. Pager: cache, allocation, freelist, fsync
3. Key encoding and row encoding, with the ordering property test
4. B+Tree search, insert, split, leaf-chain scan, `check_invariants()`
5. B+Tree delete, borrow, merge
6. Catalog: schema persistence, `CREATE TABLE` / `DROP TABLE`
7. Lexer, AST, parser
8. Planner and executor for `CREATE`, `INSERT`, `SELECT` with `SeqScan` + `Filter`
9. `TableSeek`, `Project`, `Sort`, `Limit`
10. `UPDATE` and `DELETE`
11. `CREATE INDEX`, `IndexSeek`, index maintenance, index equivalence tests
12. REPL and meta-commands, including `.btree`
13. Persistence and durability tests; README documenting every stated limitation

## Success criteria

The project is done when:

- A table of 100,000 rows can be created, populated, queried, updated, and deleted
  through SQL typed at the REPL
- The database file survives process exit and reopens correctly
- `.btree` shows a tree of height > 1 that satisfies `check_invariants()`
- A query on an indexed column measurably touches fewer pages than the same query
  without the index, demonstrated by `.stats`
- Every limitation named in this document is stated in the README
```

