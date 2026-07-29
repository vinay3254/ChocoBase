# SQL Database Engine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a single-file, single-process SQL database engine in Rust from scratch: a 4KB-page pager, a B+Tree used for both table storage and secondary indexes, a hand-written SQL parser, a rule-based planner, and a Volcano-model executor, driven from a REPL.

**Architecture:** Six layers, each usable and testable without the layer above it: Pager → B+Tree → Catalog → Parser → Planner → Executor, with a REPL on top. Keys are order-preserving byte strings so one B+Tree implementation serves both table storage (key = primary key, payload = row) and secondary indexes (key = indexed value + primary key, payload = empty).

**Tech Stack:** Rust 2021, `thiserror`, `rustyline`. Dev-only: `proptest`, `tempfile`. No `serde`, no `sqlparser` — hand-rolled on purpose.

## Global Constraints

- Page size is fixed at 4096 bytes (`PAGE_SIZE` constant), never configurable at runtime.
- A single row must fit in one page; oversized rows are a `RowTooLarge` error, never truncated or chained across overflow pages.
- Keys are order-preserving byte strings (`Vec<u8>`) such that `memcmp` order equals SQL order; `INTEGER` uses big-endian sign-flipped 8 bytes, `TEXT` uses UTF-8 + `0x00` terminator, `BOOLEAN` is one byte. Because the `TEXT` terminator is a literal `0x00` byte, a `TEXT` value containing an interior NUL character would break order-preservation for composite keys built from it (a value ending exactly at the embedded NUL becomes a byte-prefix of one that continues past it). `TEXT` values may not contain an interior NUL; this is validated at insert (Task 27's `insert_row`, which every mutating write path — `INSERT` and, via `update_row`, `UPDATE` — routes through) and rejected as `ExecError::InvalidValue`.
- Row payload encoding (little-endian, for speed) is always distinct from key encoding (order-preserving, big-endian) — never conflate the two.
- Every table must declare exactly one `PRIMARY KEY` column; there is no implicit rowid. The primary key column is always `NOT NULL`, regardless of how it was declared.
- Indexed columns must be `NOT NULL`; `CREATE INDEX` on a nullable column is a rejected, clearly reported error.
- `NULL` compares as false in every comparison operator (`=`, `<`, etc.) rather than SQL's three-valued unknown; `IS NULL` / `IS NOT NULL` are the only way to test for it. This is a documented, intentional simplification.
- No parent pointers anywhere in the B+Tree; the cursor's descent path (`Vec<u32>` of page numbers) is the only mechanism for walking back up during split/merge.
- Every mutating B+Tree operation (`insert`, `delete`) can change the tree's root page number (via split, merge, or root collapse). Whichever layer initiated the operation is responsible for reading `BTree::root()` afterward and persisting it back into the catalog. Never assume a root page number stays fixed across a mutation.
- Durability is autocommit-per-statement: flush all dirty pages, then `sync_all()`. There is no rollback — a statement or a crash can leave partial writes. This must be stated in the README, not hidden.
- Dependencies are limited to those named in the Tech Stack line above. Do not add a crate to solve a problem this plan asks you to hand-roll (LRU eviction, SQL parsing, page serialization).

---

## Task 1: Project scaffold

**Files:**
- Create: `Cargo.toml`
- Create: `src/lib.rs`
- Create: `src/main.rs`
- Create: `src/error.rs`
- Create: `src/storage.rs`
- Create: `src/btree.rs`
- Create: `src/types.rs`
- Create: `src/catalog.rs`
- Create: `src/sql.rs`
- Create: `src/plan.rs`
- Create: `src/exec.rs`
- Create: `src/engine.rs`
- Create: `src/repl.rs`

**Interfaces:**
- Produces: the module tree every later task attaches to. `error::{StorageError, BTreeError, ParseError, PlanError, ExecError, DbError, Result}`.

- [ ] **Step 1: Write `Cargo.toml`**

```toml
[package]
name = "dbengine"
version = "0.1.0"
edition = "2021"

[dependencies]
thiserror = "1"
rustyline = "14"

[dev-dependencies]
proptest = "1"
tempfile = "3"
```

- [ ] **Step 2: Write `src/error.rs`**

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a valid database file")]
    NotADatabase,
    #[error("corrupt page {0}: {1}")]
    CorruptPage(u32, String),
    #[error("page {0} out of range")]
    PageOutOfRange(u32),
}

#[derive(Debug, Error)]
pub enum BTreeError {
    #[error("row too large: {0} bytes exceeds page size {1}")]
    RowTooLarge(usize, usize),
    #[error("duplicate key")]
    DuplicateKey,
    #[error(transparent)]
    Storage(#[from] StorageError),
}

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("parse error at byte {offset}: {message}")]
    Syntax { offset: usize, message: String },
}

#[derive(Debug, Error)]
pub enum PlanError {
    #[error("no such table: {0}")]
    NoSuchTable(String),
    #[error("no such column: {0}")]
    NoSuchColumn(String),
    #[error("no such index: {0}")]
    NoSuchIndex(String),
    #[error("table already exists: {0}")]
    TableAlreadyExists(String),
    #[error("index already exists: {0}")]
    IndexAlreadyExists(String),
    #[error("invalid schema: {0}")]
    InvalidSchema(String),
}

#[derive(Debug, Error)]
pub enum ExecError {
    #[error("NOT NULL constraint failed: {0}")]
    NotNullViolation(String),
    #[error("duplicate primary key")]
    DuplicatePrimaryKey,
    #[error("invalid value: {0}")]
    InvalidValue(String),
    #[error(transparent)]
    BTree(#[from] BTreeError),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

#[derive(Debug, Error)]
pub enum DbError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    BTree(#[from] BTreeError),
    #[error(transparent)]
    Parse(#[from] ParseError),
    #[error(transparent)]
    Plan(#[from] PlanError),
    #[error(transparent)]
    Exec(#[from] ExecError),
}

pub type Result<T> = std::result::Result<T, DbError>;
```

- [ ] **Step 3: Write module declaration files, and an empty stub for every submodule they declare**

Each of these files declares `pub mod` for submodules that later tasks create. A `pub mod foo;` line does not compile until `foo.rs` exists — even as an empty file — so every submodule referenced here must exist now as a zero-byte placeholder, or `cargo build` (and therefore `cargo test`, which compiles the whole crate before running anything) will fail for every task between now and whichever task finally creates that file.

`src/storage.rs`:
```rust
pub mod page;
pub mod header;
pub mod pager;
```

`src/btree.rs`:
```rust
pub mod node;
pub mod cursor;
pub mod tree;
```

`src/types.rs`:
```rust
pub mod value;
pub mod row;
pub mod schema;
```

`src/sql.rs`:
```rust
pub mod token;
pub mod lexer;
pub mod ast;
pub mod parser;
```

`src/plan.rs`:
```rust
pub mod expr;
pub mod planner;
```

Note: this is **not** `pub mod expr; pub mod plan; pub mod planner;`. There is no `src/plan/plan.rs` anywhere in this plan — the "plan node tree" from the original design spec's module layout was folded directly into `Box<dyn Operator>` chains built by `planner::build_select_plan` (Task 28 onward), so a standalone plan-node IR file was never needed. Declaring `pub mod plan;` here would be a permanently dangling module reference.

`src/exec.rs`:
```rust
pub mod scan;
pub mod filter;
pub mod project;
pub mod sort;
pub mod limit;
pub mod mutate;
```

Note: this is **only** the six `pub mod` lines — no `Operator` trait yet, and no `use` statements. The natural design (from the original spec) put the `Operator` trait directly in `exec.rs`, but that trait's signature needs `Pager` (from Task 3) and `Value` (from Task 5), neither of which exists yet in Task 1. Defining it here would leave `exec.rs` with unresolved imports for the several tasks between now and Task 5, breaking `cargo test` crate-wide the whole time. Task 25 (the first task that actually implements an `Operator`, via `SeqScan`) adds the trait definition to this file instead, at the point where its dependencies are finally satisfied.

`src/catalog.rs`:
```rust
pub mod record;
```

Now create an empty (zero-byte) file for every submodule referenced above — these are placeholders; each later task's own "Create: `src/.../foo.rs`" instruction means "replace this empty placeholder with real content," not "create a brand-new file":

```
src/storage/page.rs
src/storage/header.rs
src/storage/pager.rs
src/btree/node.rs
src/btree/cursor.rs
src/btree/tree.rs
src/types/value.rs
src/types/row.rs
src/types/schema.rs
src/catalog/record.rs
src/sql/token.rs
src/sql/lexer.rs
src/sql/ast.rs
src/sql/parser.rs
src/plan/expr.rs
src/plan/planner.rs
src/exec/scan.rs
src/exec/filter.rs
src/exec/project.rs
src/exec/sort.rs
src/exec/limit.rs
src/exec/mutate.rs
```

- [ ] **Step 4: Write `src/lib.rs`**

```rust
pub mod error;
pub mod storage;
pub mod btree;
pub mod types;
pub mod catalog;
pub mod sql;
pub mod plan;
pub mod exec;
pub mod engine;

pub use error::{DbError, Result};
pub use engine::Database;
```

- [ ] **Step 5: Write placeholder `src/engine.rs` and `src/repl.rs`**

`src/engine.rs`:
```rust
pub struct Database;
```

`src/repl.rs` — leave empty for now (filled in Task 38).

- [ ] **Step 6: Write `src/main.rs`**

```rust
fn main() {
    println!("dbengine scaffold");
}
```

- [ ] **Step 7: Write `.gitignore`**

```
/target
```

This crate has a binary target (`src/main.rs`), so the standard Rust convention is to commit `Cargo.lock` for reproducible builds — do not gitignore it.

- [ ] **Step 8: Verify the project builds and tests run**

Run: `cargo build`
Expected: compiles with no errors (unused-code warnings are fine at this stage — every submodule is still an empty placeholder).

Run: `cargo test`
Expected: compiles and runs with 0 tests, 0 failures. This confirms the whole module tree resolves — the thing that actually broke the first time this task was attempted without a working Rust toolchain to check it against.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "Scaffold project structure and error types"
```

---

## Task 2: Page and header primitives

**Files:**
- Create: `src/storage/page.rs`
- Create: `src/storage/header.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `page::{PAGE_SIZE, PAGE_TYPE_INTERNAL, PAGE_TYPE_LEAF, PAGE_TYPE_FREE, Page}` with `Page::zeroed()`, `read_u8/write_u8`, `read_u16/write_u16`, `read_u32/write_u32`, `read_bytes/write_bytes`. `header::{MAGIC, Header}` with `Header::new()`, `Header::read_from(&Page) -> Result<Header, StorageError>`, `Header::write_to(&self, &mut Page)`.

- [ ] **Step 1: Write the failing test for `Page`**

Add to `src/storage/page.rs`:

```rust
pub const PAGE_SIZE: usize = 4096;
pub const PAGE_TYPE_INTERNAL: u8 = 1;
pub const PAGE_TYPE_LEAF: u8 = 2;
pub const PAGE_TYPE_FREE: u8 = 3;

#[derive(Clone)]
pub struct Page {
    pub data: [u8; PAGE_SIZE],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_write_roundtrip() {
        let mut page = Page::zeroed();
        page.write_u8(0, 7);
        page.write_u16(1, 1000);
        page.write_u32(3, 70000);
        page.write_bytes(7, b"hello");
        assert_eq!(page.read_u8(0), 7);
        assert_eq!(page.read_u16(1), 1000);
        assert_eq!(page.read_u32(3), 70000);
        assert_eq!(page.read_bytes(7, 5), b"hello");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test page::tests::read_write_roundtrip`
Expected: FAIL — `Page::zeroed`, `write_u8`, etc. not defined.

- [ ] **Step 3: Implement `Page`**

Add above the test module in `src/storage/page.rs`:

```rust
impl Page {
    pub fn zeroed() -> Self {
        Page { data: [0u8; PAGE_SIZE] }
    }

    pub fn read_u8(&self, offset: usize) -> u8 {
        self.data[offset]
    }

    pub fn write_u8(&mut self, offset: usize, v: u8) {
        self.data[offset] = v;
    }

    pub fn read_u16(&self, offset: usize) -> u16 {
        u16::from_be_bytes([self.data[offset], self.data[offset + 1]])
    }

    pub fn write_u16(&mut self, offset: usize, v: u16) {
        self.data[offset..offset + 2].copy_from_slice(&v.to_be_bytes());
    }

    pub fn read_u32(&self, offset: usize) -> u32 {
        u32::from_be_bytes(self.data[offset..offset + 4].try_into().unwrap())
    }

    pub fn write_u32(&mut self, offset: usize, v: u32) {
        self.data[offset..offset + 4].copy_from_slice(&v.to_be_bytes());
    }

    pub fn read_bytes(&self, offset: usize, len: usize) -> &[u8] {
        &self.data[offset..offset + len]
    }

    pub fn write_bytes(&mut self, offset: usize, bytes: &[u8]) {
        self.data[offset..offset + bytes.len()].copy_from_slice(bytes);
    }

    pub fn page_type(&self) -> u8 {
        self.read_u8(0)
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test page::tests::read_write_roundtrip`
Expected: PASS

- [ ] **Step 5: Write the failing test for `Header`**

Write `src/storage/header.rs`:

```rust
use crate::error::StorageError;
use crate::storage::page::{Page, PAGE_SIZE};

pub const MAGIC: &[u8; 8] = b"MINIDB\0\x01";

pub struct Header {
    pub page_size: u16,
    pub page_count: u32,
    pub freelist_head: u32,
    pub catalog_root: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrip() {
        let header = Header {
            page_size: PAGE_SIZE as u16,
            page_count: 5,
            freelist_head: 3,
            catalog_root: 1,
        };
        let mut page = Page::zeroed();
        header.write_to(&mut page);
        let read_back = Header::read_from(&page).unwrap();
        assert_eq!(read_back.page_count, 5);
        assert_eq!(read_back.freelist_head, 3);
        assert_eq!(read_back.catalog_root, 1);
    }

    #[test]
    fn rejects_bad_magic() {
        let page = Page::zeroed();
        assert!(matches!(Header::read_from(&page), Err(StorageError::NotADatabase)));
    }
}
```

- [ ] **Step 6: Run test to verify it fails**

Run: `cargo test header::tests`
Expected: FAIL — `Header::write_to` / `read_from` / `new` not defined.

- [ ] **Step 7: Implement `Header`**

Add above the test module:

```rust
impl Header {
    pub fn new() -> Self {
        Header {
            page_size: PAGE_SIZE as u16,
            page_count: 1,
            freelist_head: 0,
            catalog_root: 0,
        }
    }

    pub fn read_from(page: &Page) -> Result<Header, StorageError> {
        if &page.data[0..8] != MAGIC {
            return Err(StorageError::NotADatabase);
        }
        Ok(Header {
            page_size: page.read_u16(8),
            page_count: page.read_u32(10),
            freelist_head: page.read_u32(14),
            catalog_root: page.read_u32(18),
        })
    }

    pub fn write_to(&self, page: &mut Page) {
        page.write_bytes(0, MAGIC);
        page.write_u16(8, self.page_size);
        page.write_u32(10, self.page_count);
        page.write_u32(14, self.freelist_head);
        page.write_u32(18, self.catalog_root);
    }
}
```

- [ ] **Step 8: Run tests to verify they pass**

Run: `cargo test storage::page storage::header`
Expected: PASS (3 tests)

- [ ] **Step 9: Commit**

```bash
git add src/storage/page.rs src/storage/header.rs
git commit -m "Add page primitives and file header encoding"
```

---

## Task 3: Pager — create/open, cache, get_page/get_page_mut

**Files:**
- Create: `src/storage/pager.rs`

**Interfaces:**
- Consumes: `page::{Page, PAGE_SIZE}`, `header::Header`, `StorageError`.
- Produces: `pager::Pager` with `Pager::create(&Path) -> Result<Pager, StorageError>`, `Pager::open(&Path) -> Result<Pager, StorageError>`, `get_page(&mut self, u32) -> Result<&Page, StorageError>`, `get_page_mut(&mut self, u32) -> Result<&mut Page, StorageError>`, `catalog_root(&self) -> u32`, `set_catalog_root(&mut self, u32) -> Result<(), StorageError>`. Later tasks (4) add `allocate_page`, `free_page`, `flush`, `stats`.

- [ ] **Step 1: Write the failing test**

Write `src/storage/pager.rs`:

```rust
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::error::StorageError;
use crate::storage::header::Header;
use crate::storage::page::{Page, PAGE_SIZE};

const DEFAULT_CACHE_CAPACITY: usize = 256;

pub struct Pager {
    file: File,
    cache: HashMap<u32, Page>,
    recency: VecDeque<u32>,
    dirty: HashSet<u32>,
    header: Header,
    capacity: usize,
    pub pages_read: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn create_then_reopen_preserves_header() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path();
        {
            let mut pager = Pager::create(path).unwrap();
            pager.set_catalog_root(9).unwrap();
        }
        let pager = Pager::open(path).unwrap();
        assert_eq!(pager.catalog_root(), 9);
    }

    #[test]
    fn get_page_mut_then_get_page_sees_write() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        // page 1 doesn't exist on disk yet in this test; write through cache directly.
        {
            let page = pager.get_page_mut(1).unwrap();
            page.write_u8(0, 42);
        }
        let page = pager.get_page(1).unwrap();
        assert_eq!(page.read_u8(0), 42);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test storage::pager::tests`
Expected: FAIL — `Pager::create` etc. not defined.

- [ ] **Step 3: Implement `Pager` create/open and page access**

Add above the test module:

```rust
impl Pager {
    pub fn create(path: &Path) -> Result<Self, StorageError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        let mut pager = Pager {
            file,
            cache: HashMap::new(),
            recency: VecDeque::new(),
            dirty: HashSet::new(),
            header: Header::new(),
            capacity: DEFAULT_CACHE_CAPACITY,
            pages_read: 0,
        };
        pager.flush_header()?;
        pager.file.sync_all()?;
        Ok(pager)
    }

    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let mut file = OpenOptions::new().read(true).write(true).open(path)?;
        let mut buf = [0u8; PAGE_SIZE];
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut buf)?;
        let page = Page { data: buf };
        let header = Header::read_from(&page)?;
        Ok(Pager {
            file,
            cache: HashMap::new(),
            recency: VecDeque::new(),
            dirty: HashSet::new(),
            header,
            capacity: DEFAULT_CACHE_CAPACITY,
            pages_read: 0,
        })
    }

    pub fn catalog_root(&self) -> u32 {
        self.header.catalog_root
    }

    pub fn set_catalog_root(&mut self, root: u32) -> Result<(), StorageError> {
        self.header.catalog_root = root;
        self.flush_header()
    }

    fn flush_header(&mut self) -> Result<(), StorageError> {
        let mut page = Page::zeroed();
        self.header.write_to(&mut page);
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&page.data)?;
        Ok(())
    }

    fn read_page_from_disk(&mut self, no: u32) -> Result<Page, StorageError> {
        let mut buf = [0u8; PAGE_SIZE];
        self.file.seek(SeekFrom::Start(no as u64 * PAGE_SIZE as u64))?;
        self.file.read_exact(&mut buf)?;
        Ok(Page { data: buf })
    }

    fn write_page_to_disk(&mut self, no: u32) -> Result<(), StorageError> {
        let page = self.cache.get(&no).expect("dirty page must be cached");
        self.file.seek(SeekFrom::Start(no as u64 * PAGE_SIZE as u64))?;
        self.file.write_all(&page.data)?;
        Ok(())
    }

    fn touch(&mut self, no: u32) {
        self.recency.retain(|&x| x != no);
        self.recency.push_back(no);
    }

    fn evict_if_needed(&mut self) -> Result<(), StorageError> {
        while self.cache.len() >= self.capacity {
            match self.recency.front().copied() {
                Some(victim) => {
                    if self.dirty.contains(&victim) {
                        self.write_page_to_disk(victim)?;
                        self.dirty.remove(&victim);
                    }
                    self.cache.remove(&victim);
                    self.recency.pop_front();
                }
                None => break,
            }
        }
        Ok(())
    }

    fn ensure_loaded(&mut self, no: u32) -> Result<(), StorageError> {
        if !self.cache.contains_key(&no) {
            self.evict_if_needed()?;
            let page = self.read_page_from_disk(no)?;
            self.cache.insert(no, page);
            self.pages_read += 1;
        }
        self.touch(no);
        Ok(())
    }

    pub fn get_page(&mut self, no: u32) -> Result<&Page, StorageError> {
        self.ensure_loaded(no)?;
        Ok(self.cache.get(&no).unwrap())
    }

    pub fn get_page_mut(&mut self, no: u32) -> Result<&mut Page, StorageError> {
        self.ensure_loaded(no)?;
        self.dirty.insert(no);
        Ok(self.cache.get_mut(&no).unwrap())
    }
}
```

Note: `get_page_mut(1)` in the test works even though page 1 was never written to disk, because `read_page_from_disk` will try to seek/read past EOF and fail — so this test actually needs page 1 to exist. Fix the test setup instead of the implementation:

Replace the second test with:

```rust
    #[test]
    fn get_page_mut_then_get_page_sees_write() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let no = pager.allocate_page_for_test();
        {
            let page = pager.get_page_mut(no).unwrap();
            page.write_u8(0, 42);
        }
        let page = pager.get_page(no).unwrap();
        assert_eq!(page.read_u8(0), 42);
    }
```

Add a minimal test-only helper right in the `impl Pager` block above (this is superseded by the real `allocate_page` in Task 4, but the pager needs *some* way to grow the file for this test to be meaningful):

```rust
    #[cfg(test)]
    pub fn allocate_page_for_test(&mut self) -> u32 {
        let no = self.header.page_count;
        self.header.page_count += 1;
        self.flush_header().unwrap();
        self.file.set_len((no as u64 + 1) * PAGE_SIZE as u64).unwrap();
        let page = Page::zeroed();
        self.cache.insert(no, page);
        self.dirty.insert(no);
        self.touch(no);
        no
    }
```

This test-only helper is deleted in Task 4 once the real `allocate_page` exists — Task 4's Step 1 removes it as part of writing Task 4's own test.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test storage::pager::tests`
Expected: PASS (2 tests)

- [ ] **Step 5: Commit**

```bash
git add src/storage/pager.rs
git commit -m "Add pager with bounded LRU cache and header persistence"
```

---

## Task 4: Pager — allocate_page/free_page (freelist), flush/fsync, stats

**Files:**
- Modify: `src/storage/pager.rs`

**Interfaces:**
- Consumes: everything from Task 3.
- Produces: `allocate_page(&mut self) -> Result<u32, StorageError>`, `free_page(&mut self, u32) -> Result<(), StorageError>`, `flush(&mut self) -> Result<(), StorageError>`, `PagerStats { page_count, freelist_head, cached_pages, pages_read }`, `stats(&self) -> PagerStats`, `reset_read_counter(&mut self)`.

- [ ] **Step 1: Remove the test-only helper and write the failing test**

In `src/storage/pager.rs`, delete the `#[cfg(test)] pub fn allocate_page_for_test` method added in Task 3, and update `get_page_mut_then_get_page_sees_write` to call `pager.allocate_page().unwrap()` instead of `pager.allocate_page_for_test()`.

Add new tests to the `tests` module:

```rust
    #[test]
    fn allocate_extends_file_and_free_reuses_page() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let a = pager.allocate_page().unwrap();
        let b = pager.allocate_page().unwrap();
        assert_ne!(a, b);
        pager.free_page(a).unwrap();
        let c = pager.allocate_page().unwrap();
        assert_eq!(c, a, "freed page should be reused before extending the file");
    }

    #[test]
    fn flush_persists_dirty_pages_across_reopen() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path();
        let no;
        {
            let mut pager = Pager::create(path).unwrap();
            no = pager.allocate_page().unwrap();
            pager.get_page_mut(no).unwrap().write_u8(5, 77);
            pager.flush().unwrap();
        }
        let mut pager = Pager::open(path).unwrap();
        assert_eq!(pager.get_page(no).unwrap().read_u8(5), 77);
    }

    #[test]
    fn stats_report_page_count_and_reads() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let no = pager.allocate_page().unwrap();
        pager.flush().unwrap();
        pager.reset_read_counter();
        let _ = pager.get_page(no); // still cached, no disk read
        let stats = pager.stats();
        assert_eq!(stats.pages_read, 0);
        assert!(stats.page_count >= 2);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test storage::pager::tests`
Expected: FAIL — `allocate_page`, `free_page`, `flush`, `stats`, `reset_read_counter` not defined; compile error from the deleted helper if not fully removed.

- [ ] **Step 3: Implement allocate/free/flush/stats**

Add to the `impl Pager` block:

```rust
    pub fn allocate_page(&mut self) -> Result<u32, StorageError> {
        let no = if self.header.freelist_head != 0 {
            let free_no = self.header.freelist_head;
            self.ensure_loaded(free_no)?;
            let next = self.cache.get(&free_no).unwrap().read_u32(0);
            self.header.freelist_head = next;
            self.flush_header()?;
            free_no
        } else {
            let no = self.header.page_count;
            self.header.page_count += 1;
            self.flush_header()?;
            self.file.set_len(self.header.page_count as u64 * PAGE_SIZE as u64)?;
            no
        };
        self.evict_if_needed()?;
        self.cache.insert(no, Page::zeroed());
        self.dirty.insert(no);
        self.touch(no);
        Ok(no)
    }

    pub fn free_page(&mut self, no: u32) -> Result<(), StorageError> {
        let mut page = Page::zeroed();
        page.write_u32(0, self.header.freelist_head);
        self.evict_if_needed()?;
        self.cache.insert(no, page);
        self.dirty.insert(no);
        self.touch(no);
        self.header.freelist_head = no;
        self.flush_header()
    }

    pub fn flush(&mut self) -> Result<(), StorageError> {
        let dirty_pages: Vec<u32> = self.dirty.iter().copied().collect();
        for no in dirty_pages {
            self.write_page_to_disk(no)?;
            self.dirty.remove(&no);
        }
        self.file.sync_all()?;
        Ok(())
    }

    pub fn reset_read_counter(&mut self) {
        self.pages_read = 0;
    }

    pub fn stats(&self) -> PagerStats {
        PagerStats {
            page_count: self.header.page_count,
            freelist_head: self.header.freelist_head,
            cached_pages: self.cache.len(),
            pages_read: self.pages_read,
        }
    }
```

Add above `impl Pager`:

```rust
#[derive(Debug, Clone, Copy)]
pub struct PagerStats {
    pub page_count: u32,
    pub freelist_head: u32,
    pub cached_pages: usize,
    pub pages_read: u64,
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test storage::pager::tests`
Expected: PASS (5 tests)

- [ ] **Step 5: Commit**

```bash
git add src/storage/pager.rs
git commit -m "Add page allocation, freelist reuse, flush, and pager stats"
```

---

## Task 5: Value type and order-preserving key encoding

**Files:**
- Create: `src/types/value.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `value::{Value, ColumnType}` where `Value` is `Integer(i64) | Text(String) | Boolean(bool) | Null`; `encode_key(&Value) -> Vec<u8>`; `encode_composite_key(&[Value]) -> Vec<u8>`; `sql_cmp(&Value, &Value) -> std::cmp::Ordering` (panics on mismatched non-Null variants — only called where types are already known to match); `sql_cmp_nullable(&Value, &Value) -> std::cmp::Ordering` (Null sorts first, never panics).

- [ ] **Step 1: Write the failing unit tests**

Write `src/types/value.rs`:

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnType {
    Integer,
    Text,
    Boolean,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Integer(i64),
    Text(String),
    Boolean(bool),
    Null,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_encoding_preserves_order_across_sign() {
        let neg = encode_key(&Value::Integer(-5));
        let zero = encode_key(&Value::Integer(0));
        let pos = encode_key(&Value::Integer(5));
        assert!(neg < zero);
        assert!(zero < pos);
    }

    #[test]
    fn integer_encoding_extremes() {
        let min = encode_key(&Value::Integer(i64::MIN));
        let max = encode_key(&Value::Integer(i64::MAX));
        assert!(min < max);
    }

    #[test]
    fn text_encoding_preserves_lexicographic_order() {
        let a = encode_key(&Value::Text("apple".into()));
        let b = encode_key(&Value::Text("banana".into()));
        let ab = encode_key(&Value::Text("app".into()));
        assert!(a < b);
        assert!(ab < a, "a proper prefix must sort before its extension");
    }

    #[test]
    fn boolean_encoding_orders_false_before_true() {
        assert!(encode_key(&Value::Boolean(false)) < encode_key(&Value::Boolean(true)));
    }

    #[test]
    fn composite_key_concatenates_parts_in_order() {
        let k1 = encode_composite_key(&[Value::Integer(1), Value::Text("x".into())]);
        let k2 = encode_composite_key(&[Value::Integer(1), Value::Text("y".into())]);
        assert!(k1 < k2);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test types::value::tests`
Expected: FAIL — `encode_key` not defined.

- [ ] **Step 3: Implement encoding**

Add above the test module:

```rust
pub fn encode_key(v: &Value) -> Vec<u8> {
    match v {
        Value::Integer(i) => {
            let flipped = (*i as u64) ^ 0x8000_0000_0000_0000;
            flipped.to_be_bytes().to_vec()
        }
        Value::Boolean(b) => vec![if *b { 1 } else { 0 }],
        Value::Text(s) => {
            let mut out = s.as_bytes().to_vec();
            out.push(0);
            out
        }
        Value::Null => panic!("NULL cannot be encoded as a key"),
    }
}

pub fn encode_composite_key(parts: &[Value]) -> Vec<u8> {
    let mut out = Vec::new();
    for p in parts {
        out.extend(encode_key(p));
    }
    out
}

pub fn sql_cmp(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Integer(x), Value::Integer(y)) => x.cmp(y),
        (Value::Text(x), Value::Text(y)) => x.cmp(y),
        (Value::Boolean(x), Value::Boolean(y)) => x.cmp(y),
        _ => panic!("cannot compare values of different types: {:?} vs {:?}", a, b),
    }
}

pub fn sql_cmp_nullable(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Null, Value::Null) => std::cmp::Ordering::Equal,
        (Value::Null, _) => std::cmp::Ordering::Less,
        (_, Value::Null) => std::cmp::Ordering::Greater,
        _ => sql_cmp(a, b),
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test types::value::tests`
Expected: PASS (5 tests)

- [ ] **Step 5: Write the property test**

Add a `proptest` block to `src/types/value.rs` (still inside `#[cfg(test)] mod tests`, add below the unit tests):

```rust
    use proptest::prelude::*;

    fn arb_integer() -> impl Strategy<Value = Value> {
        any::<i64>().prop_map(Value::Integer)
    }

    fn arb_text() -> impl Strategy<Value = Value> {
        "[a-zA-Z0-9]{0,12}".prop_map(Value::Text)
    }

    proptest! {
        #[test]
        fn integer_key_order_matches_sql_order(a in any::<i64>(), b in any::<i64>()) {
            let va = Value::Integer(a);
            let vb = Value::Integer(b);
            let byte_order = encode_key(&va).cmp(&encode_key(&vb));
            let sql_order = sql_cmp(&va, &vb);
            prop_assert_eq!(byte_order, sql_order);
        }

        #[test]
        fn text_key_order_matches_sql_order(a in "[a-zA-Z0-9]{0,12}", b in "[a-zA-Z0-9]{0,12}") {
            let va = Value::Text(a);
            let vb = Value::Text(b);
            let byte_order = encode_key(&va).cmp(&encode_key(&vb));
            let sql_order = sql_cmp(&va, &vb);
            prop_assert_eq!(byte_order, sql_order);
        }
    }
```

Remove the unused `arb_integer`/`arb_text` helper functions if the linter flags them — they're illustrative and not required once the `proptest!` macro generates values inline via `any::<i64>()` and the string regex strategy directly.

- [ ] **Step 6: Run property tests to verify they pass**

Run: `cargo test types::value::tests`
Expected: PASS (7 tests total, property tests run 256 cases each by default)

- [ ] **Step 7: Commit**

```bash
git add src/types/value.rs
git commit -m "Add Value type and order-preserving key encoding with property tests"
```

---

## Task 6: Schema types

**Files:**
- Create: `src/types/schema.rs`

**Interfaces:**
- Consumes: `value::ColumnType`.
- Produces: `schema::{Column, TableSchema, IndexSchema}`. `TableSchema::primary_key_index(&self) -> usize`, `TableSchema::column_index(&self, name: &str) -> Option<usize>`.

- [ ] **Step 1: Write the failing test**

Write `src/types/schema.rs`:

```rust
use crate::types::value::ColumnType;

#[derive(Debug, Clone)]
pub struct Column {
    pub name: String,
    pub ty: ColumnType,
    pub not_null: bool,
    pub is_primary_key: bool,
}

#[derive(Debug, Clone)]
pub struct TableSchema {
    pub name: String,
    pub columns: Vec<Column>,
    pub root_page: u32,
}

#[derive(Debug, Clone)]
pub struct IndexSchema {
    pub name: String,
    pub table: String,
    pub column: String,
    pub root_page: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> TableSchema {
        TableSchema {
            name: "users".into(),
            columns: vec![
                Column { name: "id".into(), ty: ColumnType::Integer, not_null: true, is_primary_key: true },
                Column { name: "name".into(), ty: ColumnType::Text, not_null: false, is_primary_key: false },
            ],
            root_page: 2,
        }
    }

    #[test]
    fn finds_primary_key_index() {
        assert_eq!(sample().primary_key_index(), 0);
    }

    #[test]
    fn finds_column_by_name() {
        assert_eq!(sample().column_index("name"), Some(1));
        assert_eq!(sample().column_index("missing"), None);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test types::schema::tests`
Expected: FAIL — methods not defined.

- [ ] **Step 3: Implement**

Add above the test module:

```rust
impl TableSchema {
    pub fn primary_key_index(&self) -> usize {
        self.columns
            .iter()
            .position(|c| c.is_primary_key)
            .expect("table schema must have exactly one primary key column")
    }

    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name == name)
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test types::schema::tests`
Expected: PASS (2 tests)

- [ ] **Step 5: Commit**

```bash
git add src/types/schema.rs
git commit -m "Add table and index schema types"
```

---

## Task 7: Row encoding and decoding

**Files:**
- Create: `src/types/row.rs`

**Interfaces:**
- Consumes: `value::Value`, `value::ColumnType`, `schema::TableSchema`.
- Produces: `row::{encode_row, decode_row}` — `encode_row(&TableSchema, &[Value]) -> Vec<u8>`, `decode_row(&TableSchema, &[u8]) -> Vec<Value>`.

- [ ] **Step 1: Write the failing test**

Write `src/types/row.rs`:

```rust
use crate::types::schema::TableSchema;
use crate::types::value::{ColumnType, Value};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::schema::Column;

    fn schema() -> TableSchema {
        TableSchema {
            name: "t".into(),
            columns: vec![
                Column { name: "id".into(), ty: ColumnType::Integer, not_null: true, is_primary_key: true },
                Column { name: "name".into(), ty: ColumnType::Text, not_null: false, is_primary_key: false },
                Column { name: "active".into(), ty: ColumnType::Boolean, not_null: false, is_primary_key: false },
            ],
        }
    }

    #[test]
    fn roundtrip_no_nulls() {
        let s = schema();
        let values = vec![Value::Integer(42), Value::Text("hi".into()), Value::Boolean(true)];
        let encoded = encode_row(&s, &values);
        let decoded = decode_row(&s, &encoded);
        assert_eq!(decoded, values);
    }

    #[test]
    fn roundtrip_with_nulls() {
        let s = schema();
        let values = vec![Value::Integer(1), Value::Null, Value::Null];
        let encoded = encode_row(&s, &values);
        let decoded = decode_row(&s, &encoded);
        assert_eq!(decoded, values);
    }
}
```

Note: `TableSchema` in this test omits `root_page` in the struct literal, but the real struct requires it. Add `root_page: 0,` to the `schema()` helper's struct literal.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test types::row::tests`
Expected: FAIL — `encode_row`/`decode_row` not defined.

- [ ] **Step 3: Implement**

Add above the test module:

```rust
pub fn encode_row(schema: &TableSchema, values: &[Value]) -> Vec<u8> {
    let bitmap_len = (schema.columns.len() + 7) / 8;
    let mut bitmap = vec![0u8; bitmap_len];
    for (i, v) in values.iter().enumerate() {
        if matches!(v, Value::Null) {
            bitmap[i / 8] |= 1 << (i % 8);
        }
    }

    let mut out = bitmap;
    for v in values {
        match v {
            Value::Null => {}
            Value::Integer(i) => out.extend(&i.to_le_bytes()),
            Value::Boolean(b) => out.push(if *b { 1 } else { 0 }),
            Value::Text(s) => {
                assert!(
                    s.len() <= u16::MAX as usize,
                    "text value of {} bytes exceeds the {}-byte length-prefix limit",
                    s.len(),
                    u16::MAX
                );
                out.extend(&(s.len() as u16).to_le_bytes());
                out.extend(s.as_bytes());
            }
        }
    }
    out
}

pub fn decode_row(schema: &TableSchema, data: &[u8]) -> Vec<Value> {
    let bitmap_len = (schema.columns.len() + 7) / 8;
    let bitmap = &data[0..bitmap_len];
    let mut pos = bitmap_len;
    let mut values = Vec::with_capacity(schema.columns.len());

    for (i, col) in schema.columns.iter().enumerate() {
        let is_null = bitmap[i / 8] & (1 << (i % 8)) != 0;
        if is_null {
            values.push(Value::Null);
            continue;
        }
        match col.ty {
            ColumnType::Integer => {
                let v = i64::from_le_bytes(data[pos..pos + 8].try_into().unwrap());
                pos += 8;
                values.push(Value::Integer(v));
            }
            ColumnType::Boolean => {
                values.push(Value::Boolean(data[pos] != 0));
                pos += 1;
            }
            ColumnType::Text => {
                let len = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap()) as usize;
                pos += 2;
                let s = String::from_utf8(data[pos..pos + len].to_vec()).unwrap();
                pos += len;
                values.push(Value::Text(s));
            }
        }
    }
    values
}
```

The `assert!` in the `Value::Text` branch matters: without it, a string of 65536 bytes or more would have its length silently wrapped into the `u16` prefix (`s.len() as u16` truncates rather than errors), while the full string bytes still get written — producing a payload whose length prefix no longer matches its actual content and corrupting every column decoded after it. In practice a string that large can never reach this function from a real `INSERT`, since the B+Tree's `RowTooLarge` check (Task 10) rejects any row exceeding the 4096-byte page long before a single text field could approach 65536 bytes — but `encode_row` is a low-level primitive with no visibility into that later, page-size-driven limit, so it should fail loudly on its own rather than silently accepting an input it can't correctly represent.

- [ ] **Step 4: Add the failing-loudly test for oversized text**

Add to the `tests` module, alongside the other tests:

```rust
    #[test]
    #[should_panic(expected = "exceeds the")]
    fn text_exceeding_u16_length_prefix_panics_instead_of_silently_truncating() {
        let s = schema();
        let huge = "x".repeat(u16::MAX as usize + 1);
        let values = vec![Value::Integer(1), Value::Text(huge), Value::Null];
        encode_row(&s, &values);
    }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test types::row::tests`
Expected: PASS (3 tests)

- [ ] **Step 6: Write the round-trip property test**

Add to the `tests` module:

```rust
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn roundtrip_random_values(
            id in any::<i64>(),
            name in prop::option::of("[a-zA-Z0-9]{0,20}"),
            active in prop::option::of(any::<bool>()),
        ) {
            let s = schema();
            let values = vec![
                Value::Integer(id),
                name.map(Value::Text).unwrap_or(Value::Null),
                active.map(Value::Boolean).unwrap_or(Value::Null),
            ];
            let encoded = encode_row(&s, &values);
            let decoded = decode_row(&s, &encoded);
            prop_assert_eq!(decoded, values);
        }
    }
```

- [ ] **Step 7: Run property test to verify it passes**

Run: `cargo test types::row::tests`
Expected: PASS (4 tests)

- [ ] **Step 8: Commit**

```bash
git add src/types/row.rs
git commit -m "Add row encoding with null bitmap and round-trip property test"
```

---

## Task 8: B+Tree node encoding (slotted page)

**Files:**
- Create: `src/btree/node.rs`

**Interfaces:**
- Consumes: `page::{Page, PAGE_SIZE, PAGE_TYPE_LEAF, PAGE_TYPE_INTERNAL}`.
- Produces: `node::{NODE_HEADER_SIZE, LeafEntry, LeafNode, InternalEntry, InternalNode}`. `LeafNode::decode(&Page) -> LeafNode`, `LeafNode::encode(&self, &mut Page)`, `LeafNode::encoded_size(&self) -> usize`. `InternalNode::decode`, `encode`, `encoded_size`, `InternalNode::child_for_key(&self, key: &[u8]) -> u32`.

Design note for the implementer: rather than mutating the on-disk slot directory and cell area byte-by-byte (which requires intricate compaction logic to avoid fragmentation), each node is decoded into a plain `Vec` of entries, mutated as ordinary Rust data, and the whole node is re-encoded from scratch. This is a deliberate simplification — the on-disk format is still a real slotted page (slot directory + cells packed from the end of the page), but in-memory mutation never has to deal with holes. This keeps the split/merge logic in Task 10 onward tractable.

- [ ] **Step 1: Write the failing test**

Write `src/btree/node.rs`:

```rust
use crate::storage::page::{Page, PAGE_SIZE, PAGE_TYPE_INTERNAL, PAGE_TYPE_LEAF};

pub const NODE_HEADER_SIZE: usize = 12;

#[derive(Debug, Clone, PartialEq)]
pub struct LeafEntry {
    pub key: Vec<u8>,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct LeafNode {
    pub entries: Vec<LeafEntry>,
    pub next_leaf: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InternalEntry {
    pub key: Vec<u8>,
    pub left_child: u32,
}

#[derive(Debug, Clone)]
pub struct InternalNode {
    pub entries: Vec<InternalEntry>,
    pub rightmost_child: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaf_encode_decode_roundtrip() {
        let node = LeafNode {
            entries: vec![
                LeafEntry { key: vec![1, 2], payload: vec![9, 9, 9] },
                LeafEntry { key: vec![1, 2, 3], payload: vec![] },
            ],
            next_leaf: 42,
        };
        let mut page = Page::zeroed();
        node.encode(&mut page);
        let decoded = LeafNode::decode(&page);
        assert_eq!(decoded.entries, node.entries);
        assert_eq!(decoded.next_leaf, 42);
        assert_eq!(page.page_type(), PAGE_TYPE_LEAF);
    }

    #[test]
    fn internal_encode_decode_roundtrip() {
        let node = InternalNode {
            entries: vec![
                InternalEntry { key: vec![5], left_child: 1 },
                InternalEntry { key: vec![10], left_child: 2 },
            ],
            rightmost_child: 3,
        };
        let mut page = Page::zeroed();
        node.encode(&mut page);
        let decoded = InternalNode::decode(&page);
        assert_eq!(decoded.entries, node.entries);
        assert_eq!(decoded.rightmost_child, 3);
        assert_eq!(page.page_type(), PAGE_TYPE_INTERNAL);
    }

    #[test]
    fn child_for_key_routes_correctly() {
        let node = InternalNode {
            entries: vec![
                InternalEntry { key: vec![5], left_child: 100 },
                InternalEntry { key: vec![10], left_child: 200 },
            ],
            rightmost_child: 300,
        };
        assert_eq!(node.child_for_key(&[1]), 100);
        assert_eq!(node.child_for_key(&[5]), 200);
        assert_eq!(node.child_for_key(&[7]), 200);
        assert_eq!(node.child_for_key(&[10]), 300);
        assert_eq!(node.child_for_key(&[99]), 300);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test btree::node::tests`
Expected: FAIL — `encode`/`decode`/`child_for_key` not defined.

- [ ] **Step 3: Implement `LeafNode`**

Add above the test module:

```rust
impl LeafNode {
    pub fn decode(page: &Page) -> Self {
        assert_eq!(page.page_type(), PAGE_TYPE_LEAF);
        let num_cells = page.read_u16(2) as usize;
        let next_leaf = page.read_u32(8);
        let mut entries = Vec::with_capacity(num_cells);
        for i in 0..num_cells {
            let slot_offset = NODE_HEADER_SIZE + i * 2;
            let cell_offset = page.read_u16(slot_offset) as usize;
            let key_len = page.read_u16(cell_offset) as usize;
            let key = page.read_bytes(cell_offset + 2, key_len).to_vec();
            let payload_len_offset = cell_offset + 2 + key_len;
            let payload_len = page.read_u16(payload_len_offset) as usize;
            let payload = page.read_bytes(payload_len_offset + 2, payload_len).to_vec();
            entries.push(LeafEntry { key, payload });
        }
        LeafNode { entries, next_leaf }
    }

    pub fn encode(&self, page: &mut Page) {
        page.data = [0u8; PAGE_SIZE];
        page.write_u8(0, PAGE_TYPE_LEAF);
        page.write_u16(2, self.entries.len() as u16);
        page.write_u32(8, self.next_leaf);

        let mut cell_end = PAGE_SIZE;
        for (i, e) in self.entries.iter().enumerate() {
            let cell_len = 2 + e.key.len() + 2 + e.payload.len();
            let cell_offset = cell_end - cell_len;
            page.write_u16(cell_offset, e.key.len() as u16);
            page.write_bytes(cell_offset + 2, &e.key);
            page.write_u16(cell_offset + 2 + e.key.len(), e.payload.len() as u16);
            page.write_bytes(cell_offset + 2 + e.key.len() + 2, &e.payload);
            page.write_u16(NODE_HEADER_SIZE + i * 2, cell_offset as u16);
            cell_end = cell_offset;
        }
        page.write_u16(4, (NODE_HEADER_SIZE + self.entries.len() * 2) as u16);
        page.write_u16(6, cell_end as u16);
    }

    pub fn encoded_size(&self) -> usize {
        let cells: usize = self.entries.iter().map(|e| 2 + e.key.len() + 2 + e.payload.len()).sum();
        NODE_HEADER_SIZE + self.entries.len() * 2 + cells
    }
}
```

- [ ] **Step 4: Implement `InternalNode`**

Add to the same file:

```rust
impl InternalNode {
    pub fn decode(page: &Page) -> Self {
        assert_eq!(page.page_type(), PAGE_TYPE_INTERNAL);
        let num_cells = page.read_u16(2) as usize;
        let rightmost_child = page.read_u32(8);
        let mut entries = Vec::with_capacity(num_cells);
        for i in 0..num_cells {
            let slot_offset = NODE_HEADER_SIZE + i * 2;
            let cell_offset = page.read_u16(slot_offset) as usize;
            let key_len = page.read_u16(cell_offset) as usize;
            let key = page.read_bytes(cell_offset + 2, key_len).to_vec();
            let left_child = page.read_u32(cell_offset + 2 + key_len);
            entries.push(InternalEntry { key, left_child });
        }
        InternalNode { entries, rightmost_child }
    }

    pub fn encode(&self, page: &mut Page) {
        page.data = [0u8; PAGE_SIZE];
        page.write_u8(0, PAGE_TYPE_INTERNAL);
        page.write_u16(2, self.entries.len() as u16);
        page.write_u32(8, self.rightmost_child);

        let mut cell_end = PAGE_SIZE;
        for (i, e) in self.entries.iter().enumerate() {
            let cell_len = 2 + e.key.len() + 4;
            let cell_offset = cell_end - cell_len;
            page.write_u16(cell_offset, e.key.len() as u16);
            page.write_bytes(cell_offset + 2, &e.key);
            page.write_u32(cell_offset + 2 + e.key.len(), e.left_child);
            page.write_u16(NODE_HEADER_SIZE + i * 2, cell_offset as u16);
            cell_end = cell_offset;
        }
        page.write_u16(4, (NODE_HEADER_SIZE + self.entries.len() * 2) as u16);
        page.write_u16(6, cell_end as u16);
    }

    pub fn encoded_size(&self) -> usize {
        let cells: usize = self.entries.iter().map(|e| 2 + e.key.len() + 4).sum();
        NODE_HEADER_SIZE + self.entries.len() * 2 + cells
    }

    pub fn child_for_key(&self, key: &[u8]) -> u32 {
        for e in &self.entries {
            if key < e.key.as_slice() {
                return e.left_child;
            }
        }
        self.rightmost_child
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test btree::node::tests`
Expected: PASS (3 tests)

- [ ] **Step 6: Commit**

```bash
git add src/btree/node.rs
git commit -m "Add slotted-page leaf and internal node encoding"
```

---

## Task 9: BTree struct, descend, and search

**Files:**
- Create: `src/btree/tree.rs`

**Interfaces:**
- Consumes: `Pager`, `node::{LeafNode, InternalNode}`, `page::{PAGE_TYPE_LEAF, PAGE_TYPE_INTERNAL}`, `BTreeError`, `StorageError`.
- Produces: `tree::BTree<'a>` with `BTree::new(&'a mut Pager, u32) -> BTree<'a>`, `root(&self) -> u32`, `search(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, BTreeError>`. Internal (crate-visible) `descend(&mut self, key: &[u8]) -> Result<(Vec<u32>, u32), BTreeError>` returning `(path_of_internal_pages, leaf_page_no)` — used by Tasks 10, 14, 15.

- [ ] **Step 1: Write the failing test**

Write `src/btree/tree.rs`. This test manually builds a two-level tree (one internal root with two leaf children) by writing pages directly, to test `search` before `insert` exists:

```rust
use crate::error::{BTreeError, StorageError};
use crate::storage::page::{PAGE_TYPE_INTERNAL, PAGE_TYPE_LEAF};
use crate::storage::pager::Pager;
use crate::btree::node::{InternalEntry, InternalNode, LeafEntry, LeafNode};

pub struct BTree<'a> {
    pager: &'a mut Pager,
    root: u32,
}

impl<'a> BTree<'a> {
    pub fn new(pager: &'a mut Pager, root: u32) -> Self {
        BTree { pager, root }
    }

    pub fn root(&self) -> u32 {
        self.root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn two_level_tree() -> (Pager, u32) {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let left = pager.allocate_page().unwrap();
        let right = pager.allocate_page().unwrap();
        let root = pager.allocate_page().unwrap();

        let left_node = LeafNode {
            entries: vec![LeafEntry { key: vec![1], payload: vec![b'a'] }],
            next_leaf: right,
        };
        left_node.encode(pager.get_page_mut(left).unwrap());

        let right_node = LeafNode {
            entries: vec![LeafEntry { key: vec![5], payload: vec![b'b'] }],
            next_leaf: 0,
        };
        right_node.encode(pager.get_page_mut(right).unwrap());

        let root_node = InternalNode {
            entries: vec![InternalEntry { key: vec![5], left_child: left }],
            rightmost_child: right,
        };
        root_node.encode(pager.get_page_mut(root).unwrap());

        pager.flush().unwrap();
        (pager, root)
    }

    #[test]
    fn search_finds_existing_key_across_levels() {
        let (mut pager, root) = two_level_tree();
        let mut bt = BTree::new(&mut pager, root);
        assert_eq!(bt.search(&[1]).unwrap(), Some(vec![b'a']));
        assert_eq!(bt.search(&[5]).unwrap(), Some(vec![b'b']));
    }

    #[test]
    fn search_returns_none_for_missing_key() {
        let (mut pager, root) = two_level_tree();
        let mut bt = BTree::new(&mut pager, root);
        assert_eq!(bt.search(&[3]).unwrap(), None);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test btree::tree::tests`
Expected: FAIL — `search` not defined.

- [ ] **Step 3: Implement `descend` and `search`**

Add to `impl<'a> BTree<'a>`:

```rust
    fn descend(&mut self, key: &[u8]) -> Result<(Vec<u32>, u32), BTreeError> {
        let mut path = Vec::new();
        let mut current = self.root;
        loop {
            let page = self.pager.get_page(current)?;
            match page.page_type() {
                PAGE_TYPE_LEAF => return Ok((path, current)),
                PAGE_TYPE_INTERNAL => {
                    let node = InternalNode::decode(page);
                    path.push(current);
                    current = node.child_for_key(key);
                }
                t => {
                    return Err(BTreeError::Storage(StorageError::CorruptPage(
                        current,
                        format!("unexpected page type {t}"),
                    )))
                }
            }
        }
    }

    pub fn search(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, BTreeError> {
        let (_, leaf_no) = self.descend(key)?;
        let page = self.pager.get_page(leaf_no)?;
        let node = LeafNode::decode(page);
        Ok(node.entries.iter().find(|e| e.key == key).map(|e| e.payload.clone()))
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test btree::tree::tests`
Expected: PASS (2 tests)

- [ ] **Step 5: Commit**

```bash
git add src/btree/tree.rs
git commit -m "Add BTree descend and search over multi-level trees"
```

---

## Task 10: BTree insert — leaf insert, leaf split, new root creation

**Files:**
- Modify: `src/btree/tree.rs`

**Interfaces:**
- Consumes: everything from Task 9.
- Produces: `insert(&mut self, key: &[u8], payload: &[u8]) -> Result<(), BTreeError>`. Root may change; caller must read `bt.root()` after calling.

This task handles inserts that start from a **single-leaf tree** (root is itself a leaf) — covering the no-split and leaf-splits-into-new-root cases. Multi-level propagation (splitting an internal node) is Task 11.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src/btree/tree.rs`:

```rust
    fn empty_tree() -> (Pager, u32) {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(root).unwrap());
        pager.flush().unwrap();
        (pager, root)
    }

    #[test]
    fn insert_then_search_finds_key() {
        let (mut pager, root) = empty_tree();
        let mut bt = BTree::new(&mut pager, root);
        bt.insert(&[1], b"one").unwrap();
        bt.insert(&[2], b"two").unwrap();
        assert_eq!(bt.search(&[1]).unwrap(), Some(b"one".to_vec()));
        assert_eq!(bt.search(&[2]).unwrap(), Some(b"two".to_vec()));
    }

    #[test]
    fn insert_duplicate_key_errors() {
        let (mut pager, root) = empty_tree();
        let mut bt = BTree::new(&mut pager, root);
        bt.insert(&[1], b"one").unwrap();
        let err = bt.insert(&[1], b"again").unwrap_err();
        assert!(matches!(err, BTreeError::DuplicateKey));
    }

    #[test]
    fn insert_row_too_large_errors() {
        let (mut pager, root) = empty_tree();
        let mut bt = BTree::new(&mut pager, root);
        let huge_payload = vec![0u8; 5000];
        let err = bt.insert(&[1], &huge_payload).unwrap_err();
        assert!(matches!(err, BTreeError::RowTooLarge(_, _)));
    }

    #[test]
    fn inserting_enough_keys_splits_root_and_changes_it() {
        let (mut pager, root) = empty_tree();
        let mut bt = BTree::new(&mut pager, root);
        // Large keys (~700 bytes) mean a leaf or internal node fills after only
        // ~5 entries, so a multi-level split cascade (through split_internal)
        // is reached in well under 100 inserts, instead of needing tens of
        // thousands of small sequential keys to overflow a 255-entry internal
        // root (verified empirically: 400 small 8-byte keys never reaches
        // split_internal at all, and 80,000 small keys does reach it but takes
        // ~38s in a debug build — far too slow to run on every `cargo test`
        // across the many remaining tasks that exercise this module). The
        // zero-padded numeric prefix preserves ascending order.
        for i in 0..100i32 {
            let key = format!("{i:06}{}", "x".repeat(700)).into_bytes();
            bt.insert(&key, b"v").unwrap();
        }
        assert_ne!(bt.root(), root, "enough inserts must force at least one split");
        for i in 0..100i32 {
            let key = format!("{i:06}{}", "x".repeat(700)).into_bytes();
            assert_eq!(
                bt.search(&key).unwrap(),
                Some(b"v".to_vec())
            );
        }
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test btree::tree::tests`
Expected: FAIL — `insert` not defined.

- [ ] **Step 3: Implement leaf insert and split**

Add to `impl<'a> BTree<'a>`:

```rust
    pub fn insert(&mut self, key: &[u8], payload: &[u8]) -> Result<(), BTreeError> {
        use crate::btree::node::NODE_HEADER_SIZE;
        use crate::storage::page::PAGE_SIZE;

        let single_entry_size = NODE_HEADER_SIZE + 2 + 2 + key.len() + 2 + payload.len();
        if single_entry_size > PAGE_SIZE {
            return Err(BTreeError::RowTooLarge(single_entry_size, PAGE_SIZE));
        }

        let (path, leaf_no) = self.descend(key)?;
        let page = self.pager.get_page(leaf_no)?;
        let mut node = LeafNode::decode(page);

        if node.entries.iter().any(|e| e.key == key) {
            return Err(BTreeError::DuplicateKey);
        }

        let pos = node.entries.partition_point(|e| e.key.as_slice() < key);
        node.entries.insert(pos, LeafEntry { key: key.to_vec(), payload: payload.to_vec() });

        if node.encoded_size() <= PAGE_SIZE {
            let page = self.pager.get_page_mut(leaf_no)?;
            node.encode(page);
            return Ok(());
        }

        let old_next = node.next_leaf;
        let mid = node.entries.len() / 2;
        let right_entries = node.entries.split_off(mid);
        let left_entries = node.entries;

        let right_no = self.pager.allocate_page()?;
        let separator = right_entries[0].key.clone();

        let left_node = LeafNode { entries: left_entries, next_leaf: right_no };
        let right_node = LeafNode { entries: right_entries, next_leaf: old_next };

        left_node.encode(self.pager.get_page_mut(leaf_no)?);
        right_node.encode(self.pager.get_page_mut(right_no)?);

        self.insert_into_parent(path, leaf_no, separator, right_no)
    }

    fn insert_into_parent(
        &mut self,
        mut path: Vec<u32>,
        left_child: u32,
        separator: Vec<u8>,
        right_child: u32,
    ) -> Result<(), BTreeError> {
        use crate::storage::page::PAGE_SIZE;

        match path.pop() {
            None => {
                let new_root_no = self.pager.allocate_page()?;
                let new_root = InternalNode {
                    entries: vec![InternalEntry { key: separator, left_child }],
                    rightmost_child: right_child,
                };
                new_root.encode(self.pager.get_page_mut(new_root_no)?);
                self.root = new_root_no;
                Ok(())
            }
            Some(parent_no) => {
                let page = self.pager.get_page(parent_no)?;
                let mut node = InternalNode::decode(page);

                if let Some(i) = node.entries.iter().position(|e| e.left_child == left_child) {
                    node.entries[i].left_child = right_child;
                    node.entries.insert(i, InternalEntry { key: separator, left_child });
                } else {
                    debug_assert_eq!(node.rightmost_child, left_child);
                    node.rightmost_child = right_child;
                    node.entries.push(InternalEntry { key: separator, left_child });
                }

                if node.encoded_size() <= PAGE_SIZE {
                    node.encode(self.pager.get_page_mut(parent_no)?);
                    return Ok(());
                }

                // Internal split: handled fully in Task 11.
                self.split_internal(path, parent_no, node)
            }
        }
    }

    fn split_internal(
        &mut self,
        _path: Vec<u32>,
        _parent_no: u32,
        _node: InternalNode,
    ) -> Result<(), BTreeError> {
        unimplemented!("implemented in Task 11")
    }
```

- [ ] **Step 4: Run tests to verify they pass — expect the split test to fail**

Run: `cargo test btree::tree::tests`
Expected: `insert_then_search_finds_key`, `insert_duplicate_key_errors`, `insert_row_too_large_errors` PASS. `inserting_enough_keys_splits_root_and_changes_it` panics with `unimplemented` — the ~700-byte keys fill both a leaf and, once enough leaf splits have promoted enough separators, the internal root itself, reaching `split_internal` in under 100 inserts. This is expected — the fourth test is only fully green after Task 11. (5 of 6 tests in the module pass; this one fails with the `unimplemented` panic, not a different error — if it fails any other way, something is wrong with the leaf-insert/split logic itself.)

- [ ] **Step 5: Commit the leaf-level insert path**

```bash
git add src/btree/tree.rs
git commit -m "Add BTree leaf insert, leaf split, and new-root creation"
```

---

## Task 11: BTree insert — internal node split (multi-level propagation)

**Files:**
- Modify: `src/btree/tree.rs`

**Interfaces:**
- Consumes: everything from Task 10.
- Produces: working `split_internal`, completing `insert` for trees of any depth.

- [ ] **Step 1: Confirm the still-failing test from Task 10**

Run: `cargo test btree::tree::tests::inserting_enough_keys_splits_root_and_changes_it`
Expected: FAIL with `unimplemented`.

- [ ] **Step 2: Implement `split_internal`**

Replace the `split_internal` stub in `src/btree/tree.rs`:

```rust
    fn split_internal(
        &mut self,
        path: Vec<u32>,
        parent_no: u32,
        mut node: InternalNode,
    ) -> Result<(), BTreeError> {
        let old_rightmost = node.rightmost_child;
        let n = node.entries.len();
        let s = n / 2;

        let mut right_entries = node.entries.split_off(s);
        let promote = right_entries.remove(0);

        let left_rightmost = promote.left_child;
        let right_no = self.pager.allocate_page()?;

        let right_node = InternalNode { entries: right_entries, rightmost_child: old_rightmost };
        let left_node = InternalNode { entries: node.entries, rightmost_child: left_rightmost };

        left_node.encode(self.pager.get_page_mut(parent_no)?);
        right_node.encode(self.pager.get_page_mut(right_no)?);

        self.insert_into_parent(path, parent_no, promote.key, right_no)
    }
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test btree::tree::tests`
Expected: PASS (7 tests, including the multi-level split test)

- [ ] **Step 4: Commit**

```bash
git add src/btree/tree.rs
git commit -m "Add internal node split, completing multi-level BTree insert"
```

---

## Task 12: check_invariants and insert property test

**Files:**
- Modify: `src/btree/tree.rs`

**Interfaces:**
- Consumes: everything from Task 11.
- Produces: `check_invariants(&mut self) -> Result<(), String>` — walks the whole tree verifying key ordering, that every leaf key falls within its subtree's bounds, and uniform leaf depth.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module:

```rust
    #[test]
    fn invariants_hold_after_many_inserts() {
        let (mut pager, root) = empty_tree();
        let mut bt = BTree::new(&mut pager, root);
        for i in 0..500i64 {
            let k = ((i * 37) % 500) as u64; // insertion order != key order
            bt.insert(&k.to_be_bytes(), format!("v{k}").as_bytes()).ok();
        }
        bt.check_invariants().unwrap();
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test btree::tree::tests::invariants_hold_after_many_inserts`
Expected: FAIL — `check_invariants` not defined.

- [ ] **Step 3: Implement `check_invariants`**

Add to `impl<'a> BTree<'a>`:

```rust
    pub fn check_invariants(&mut self) -> Result<(), String> {
        self.check_node(self.root, None, None).map(|_| ())
    }

    fn check_node(&mut self, page_no: u32, lower: Option<&[u8]>, upper: Option<&[u8]>) -> Result<usize, String> {
        let page = self.pager.get_page(page_no).map_err(|e| e.to_string())?;
        match page.page_type() {
            crate::storage::page::PAGE_TYPE_LEAF => {
                let node = LeafNode::decode(page);
                for w in node.entries.windows(2) {
                    if w[0].key >= w[1].key {
                        return Err(format!("leaf {page_no}: keys out of order"));
                    }
                }
                if let (Some(e), Some(lo)) = (node.entries.first(), lower) {
                    if e.key.as_slice() < lo {
                        return Err(format!("leaf {page_no}: key below lower bound"));
                    }
                }
                if let (Some(e), Some(hi)) = (node.entries.last(), upper) {
                    if e.key.as_slice() >= hi {
                        return Err(format!("leaf {page_no}: key at/above upper bound"));
                    }
                }
                Ok(0)
            }
            crate::storage::page::PAGE_TYPE_INTERNAL => {
                let node = InternalNode::decode(page);
                if node.entries.is_empty() {
                    return Err(format!("internal {page_no}: no entries"));
                }
                let mut depths = Vec::new();
                let mut lo = lower;
                for e in &node.entries {
                    depths.push(self.check_node(e.left_child, lo, Some(e.key.as_slice()))?);
                    lo = Some(e.key.as_slice());
                }
                depths.push(self.check_node(node.rightmost_child, lo, upper)?);
                if depths.iter().any(|&d| d != depths[0]) {
                    return Err(format!("internal {page_no}: children at unequal depth {depths:?}"));
                }
                Ok(depths[0] + 1)
            }
            t => Err(format!("page {page_no}: unexpected page type {t}")),
        }
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test btree::tree::tests::invariants_hold_after_many_inserts`
Expected: PASS

- [ ] **Step 5: Write the property test**

Add to the `tests` module:

```rust
    use proptest::prelude::*;
    use proptest::collection::vec as pvec;

    proptest! {
        #[test]
        fn insert_and_scan_matches_sorted_unique_keys(keys in pvec(0u32..2000, 1..300)) {
            let (mut pager, root) = empty_tree();
            let mut bt = BTree::new(&mut pager, root);
            let mut inserted = std::collections::BTreeSet::new();
            for k in &keys {
                let key_bytes = k.to_be_bytes();
                if bt.insert(&key_bytes, b"v").is_ok() {
                    inserted.insert(*k);
                }
            }
            bt.check_invariants().unwrap();
            for k in &inserted {
                prop_assert_eq!(bt.search(&k.to_be_bytes()).unwrap(), Some(b"v".to_vec()));
            }
        }
    }
```

- [ ] **Step 6: Run property test to verify it passes**

Run: `cargo test btree::tree::tests`
Expected: PASS (9 tests). This may take a few seconds — 256 random key sets, each checking full-tree invariants.

- [ ] **Step 7: Commit**

```bash
git add src/btree/tree.rs
git commit -m "Add check_invariants and insert/scan property test"
```

---

## Task 13: Cursor — cursor_start, cursor_seek, next

**Files:**
- Create: `src/btree/cursor.rs`
- Modify: `src/btree/tree.rs`

**Interfaces:**
- Consumes: `node::{LeafEntry, LeafNode}`, `Pager`.
- Produces: `cursor::Cursor` with `Cursor::empty() -> Cursor`, `next(&mut self, &mut Pager) -> Result<Option<(Vec<u8>, Vec<u8>)>, BTreeError>`. `BTree::cursor_start(&mut self) -> Result<Cursor, BTreeError>`, `BTree::cursor_seek(&mut self, key: &[u8]) -> Result<Cursor, BTreeError>` (first entry with key >= the given prefix/key).

- [ ] **Step 1: Write the failing test**

Write `src/btree/cursor.rs`:

```rust
use crate::error::BTreeError;
use crate::btree::node::LeafEntry;
use crate::storage::pager::Pager;

pub struct Cursor {
    index: usize,
    entries: Vec<LeafEntry>,
    next_leaf: u32,
    finished: bool,
}

#[cfg(test)]
mod tests {
    use super::super::tree::BTree;
    use crate::storage::pager::Pager;
    use crate::btree::node::LeafNode;
    use tempfile::NamedTempFile;

    fn build(keys: &[u32]) -> (Pager, u32) {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let initial_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(initial_root).unwrap());
        let final_root = {
            let mut bt = BTree::new(&mut pager, initial_root);
            for &k in keys {
                bt.insert(&k.to_be_bytes(), format!("v{k}").as_bytes()).unwrap();
            }
            bt.root()
        };
        pager.flush().unwrap();
        (pager, final_root)
    }

    #[test]
    fn full_scan_returns_all_keys_in_order() {
        let (mut pager, root) = build(&[5, 1, 3, 2, 4]);
        let mut cursor = { BTree::new(&mut pager, root).cursor_start().unwrap() };
        let mut seen = Vec::new();
        while let Some((k, _)) = cursor.next(&mut pager).unwrap() {
            seen.push(u32::from_be_bytes(k.try_into().unwrap()));
        }
        assert_eq!(seen, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn seek_positions_at_first_key_greater_or_equal() {
        let (mut pager, root) = build(&[10, 20, 30, 40]);
        let mut cursor = { BTree::new(&mut pager, root).cursor_seek(&25u32.to_be_bytes()).unwrap() };
        let (k, _) = cursor.next(&mut pager).unwrap().unwrap();
        assert_eq!(u32::from_be_bytes(k.try_into().unwrap()), 30);
    }

    #[test]
    fn seek_with_prefix_key_lands_on_first_entry_with_that_prefix() {
        // Composite index keys later in this project are variable-length byte
        // strings, and an index seek's search key is a proper prefix of the
        // full stored keys it should match (see IndexSeek, Task 35). Rust's
        // lexicographic slice Ord treats a proper prefix as "less than" its
        // extension, so seeking [1,2] against stored keys [1,1] < [1,2,3] < [1,3]
        // must land on [1,2,3] -- not skip past it as if [1,2,3] were smaller
        // than the shorter search key [1,2].
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let initial_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(initial_root).unwrap());
        let final_root = {
            let mut bt = BTree::new(&mut pager, initial_root);
            for key in [vec![1u8, 1], vec![1, 2, 3], vec![1, 3]] {
                bt.insert(&key, b"v").unwrap();
            }
            bt.root()
        };
        pager.flush().unwrap();

        let mut cursor = { BTree::new(&mut pager, final_root).cursor_seek(&[1, 2]).unwrap() };
        let (k, _) = cursor.next(&mut pager).unwrap().unwrap();
        assert_eq!(k, vec![1, 2, 3]);
    }
}
```

Note the `{ BTree::new(&mut pager, root).cursor_start().unwrap() }` pattern: `BTree` borrows `pager` mutably only for the duration of that block. Once `cursor_start`/`cursor_seek` return an owned `Cursor` (no lifetime tied to `BTree`), the borrow ends and `pager` is free to pass into `cursor.next(&mut pager)` directly on the next line. This is the same borrowing pattern every executor operator in Tasks 25 onward relies on.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test btree::cursor::tests`
Expected: FAIL — `Cursor::next`, `cursor_start`, `cursor_seek` not defined.

- [ ] **Step 3: Implement `Cursor`**

Add to `src/btree/cursor.rs`, above the test module:

```rust
impl Cursor {
    pub fn empty() -> Self {
        Cursor { index: 0, entries: Vec::new(), next_leaf: 0, finished: true }
    }

    pub fn next(&mut self, pager: &mut Pager) -> Result<Option<(Vec<u8>, Vec<u8>)>, BTreeError> {
        if self.finished {
            return Ok(None);
        }
        if self.index >= self.entries.len() {
            if self.next_leaf == 0 {
                self.finished = true;
                return Ok(None);
            }
            let page = pager.get_page(self.next_leaf)?;
            let node = crate::btree::node::LeafNode::decode(page);
            self.next_leaf = node.next_leaf;
            self.entries = node.entries;
            self.index = 0;
            return self.next(pager);
        }
        let e = self.entries[self.index].clone();
        self.index += 1;
        Ok(Some((e.key, e.payload)))
    }
}

impl Cursor {
    pub(crate) fn from_leaf(entries: Vec<LeafEntry>, index: usize, next_leaf: u32) -> Self {
        Cursor { index, entries, next_leaf, finished: false }
    }
}
```

- [ ] **Step 4: Implement `cursor_start`/`cursor_seek` on `BTree`**

Add to `impl<'a> BTree<'a>` in `src/btree/tree.rs`:

```rust
    pub fn cursor_start(&mut self) -> Result<crate::btree::cursor::Cursor, BTreeError> {
        let mut current = self.root;
        loop {
            let page = self.pager.get_page(current)?;
            match page.page_type() {
                crate::storage::page::PAGE_TYPE_LEAF => break,
                crate::storage::page::PAGE_TYPE_INTERNAL => {
                    let node = InternalNode::decode(page);
                    current = node.entries.first().map(|e| e.left_child).unwrap_or(node.rightmost_child);
                }
                t => {
                    return Err(BTreeError::Storage(StorageError::CorruptPage(
                        current,
                        format!("unexpected page type {t}"),
                    )))
                }
            }
        }
        let page = self.pager.get_page(current)?;
        let node = LeafNode::decode(page);
        Ok(crate::btree::cursor::Cursor::from_leaf(node.entries, 0, node.next_leaf))
    }

    pub fn cursor_seek(&mut self, key: &[u8]) -> Result<crate::btree::cursor::Cursor, BTreeError> {
        let (_, leaf_no) = self.descend(key)?;
        let page = self.pager.get_page(leaf_no)?;
        let node = LeafNode::decode(page);
        let start_idx = node.entries.partition_point(|e| e.key.as_slice() < key);
        Ok(crate::btree::cursor::Cursor::from_leaf(node.entries, start_idx, node.next_leaf))
    }
```

Add `pub mod cursor;` is already present from Task 1's `src/btree.rs`; no change needed there.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test btree::cursor::tests`
Expected: PASS (3 tests). The third test, `seek_with_prefix_key_lands_on_first_entry_with_that_prefix`, exists specifically to prove `cursor_seek` handles a search key that is a proper byte-string prefix of a longer stored key — this is the exact scenario Task 35's `IndexSeek` depends on later, and it is genuinely load-bearing enough to deserve its own test rather than relying on inspection alone.

- [ ] **Step 6: Run the full BTree test suite to confirm nothing broke**

Run: `cargo test btree::`
Expected: PASS (all tests from Tasks 8–13)

- [ ] **Step 7: Commit**

```bash
git add src/btree/cursor.rs src/btree/tree.rs
git commit -m "Add BTree cursor for range scans via the leaf chain"
```

---

## Task 14: BTree delete — leaf removal, borrow, and merge

**Files:**
- Modify: `src/btree/tree.rs`

**Interfaces:**
- Consumes: everything from Task 13.
- Produces: `delete(&mut self, key: &[u8]) -> Result<bool, BTreeError>` (returns whether the key existed). Root may change (via later root collapse in Task 15); caller must read `bt.root()` after calling.

Design note: rebalancing (borrow-vs-merge) decisions use a minimum **entry count**, not byte-fill percentage. `const MIN_ENTRIES: usize = 2` — a node with fewer than 2 entries is underfull and triggers a borrow or merge; a sibling with more than 2 entries "can spare" one. This is a deliberate simplification versus production databases' byte-fill-factor heuristics: it is simpler to reason about and test, and `check_invariants` (Task 12) already verifies structural correctness independent of fill factor. Splits (Tasks 10–11) remain byte-size-based, since the page has a hard byte budget — only the merge/borrow *trigger* uses entry count.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src/btree/tree.rs`:

```rust
    #[test]
    fn delete_removes_key_from_single_leaf() {
        let (mut pager, root) = empty_tree();
        let mut bt = BTree::new(&mut pager, root);
        bt.insert(&[1], b"a").unwrap();
        bt.insert(&[2], b"b").unwrap();
        assert!(bt.delete(&[1]).unwrap());
        assert_eq!(bt.search(&[1]).unwrap(), None);
        assert_eq!(bt.search(&[2]).unwrap(), Some(b"b".to_vec()));
    }

    #[test]
    fn delete_missing_key_returns_false() {
        let (mut pager, root) = empty_tree();
        let mut bt = BTree::new(&mut pager, root);
        bt.insert(&[1], b"a").unwrap();
        assert!(!bt.delete(&[99]).unwrap());
    }

    #[test]
    fn delete_causing_leaf_underflow_borrows_or_merges_and_stays_valid() {
        let (mut pager, root) = empty_tree();
        let mut bt = BTree::new(&mut pager, root);
        // Large keys (~700 bytes, same technique as Task 10's split test) so a
        // leaf holds only ~5 entries before splitting: 300 small 4-byte keys
        // never split the tree at all (they all fit in a single root leaf, so
        // delete() always takes the path.is_empty() no-rebalance-needed branch
        // -- confirmed by actually running this exact scenario), which means
        // the original small-key version of this test exercised zero
        // rebalancing despite its name. 30 large keys force several leaf
        // splits, and deleting all but the last 5 forces earlier leaves below
        // MIN_ENTRIES, genuinely exercising borrow/merge.
        let key = |i: u32| format!("{i:06}{}", "x".repeat(700)).into_bytes();
        for i in 0..30u32 {
            bt.insert(&key(i), b"v").unwrap();
        }
        for i in 0..25u32 {
            assert!(bt.delete(&key(i)).unwrap());
        }
        bt.check_invariants().unwrap();
        for i in 0..25u32 {
            assert_eq!(bt.search(&key(i)).unwrap(), None);
        }
        for i in 25..30u32 {
            assert_eq!(bt.search(&key(i)).unwrap(), Some(b"v".to_vec()));
        }
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test btree::tree::tests::delete_removes_key_from_single_leaf`
Expected: FAIL — `delete` not defined.

- [ ] **Step 3: Implement `delete` and leaf-level rebalancing**

Add to `impl<'a> BTree<'a>`:

```rust
    const MIN_ENTRIES: usize = 2;

    pub fn delete(&mut self, key: &[u8]) -> Result<bool, BTreeError> {
        let (path, leaf_no) = self.descend(key)?;
        let page = self.pager.get_page(leaf_no)?;
        let mut node = LeafNode::decode(page);
        let pos = match node.entries.iter().position(|e| e.key.as_slice() == key) {
            Some(p) => p,
            None => return Ok(false),
        };
        node.entries.remove(pos);
        node.encode(self.pager.get_page_mut(leaf_no)?);

        if path.is_empty() {
            return Ok(true);
        }
        self.rebalance_leaf(path, leaf_no)?;
        Ok(true)
    }

    fn find_siblings(&mut self, parent_no: u32, child_no: u32) -> Result<(Option<u32>, Option<u32>), BTreeError> {
        let page = self.pager.get_page(parent_no)?;
        let parent = InternalNode::decode(page);
        let children: Vec<u32> = parent
            .entries
            .iter()
            .map(|e| e.left_child)
            .chain(std::iter::once(parent.rightmost_child))
            .collect();
        let idx = children.iter().position(|&c| c == child_no).expect("child must belong to parent");
        let left = if idx > 0 { Some(children[idx - 1]) } else { None };
        let right = if idx + 1 < children.len() { Some(children[idx + 1]) } else { None };
        Ok((left, right))
    }

    fn update_separator(&mut self, parent_no: u32, left_child: u32, new_key: Vec<u8>) -> Result<(), BTreeError> {
        let page = self.pager.get_page(parent_no)?;
        let mut parent = InternalNode::decode(page);
        let i = parent.entries.iter().position(|e| e.left_child == left_child).unwrap();
        parent.entries[i].key = new_key;
        parent.encode(self.pager.get_page_mut(parent_no)?);
        Ok(())
    }

    fn rebalance_leaf(&mut self, mut path: Vec<u32>, node_no: u32) -> Result<(), BTreeError> {
        let node = LeafNode::decode(self.pager.get_page(node_no)?);
        if node.entries.len() >= Self::MIN_ENTRIES {
            return Ok(());
        }
        let parent_no = path.pop().unwrap();
        let (left_sib, right_sib) = self.find_siblings(parent_no, node_no)?;

        if let Some(right_no) = right_sib {
            let right = LeafNode::decode(self.pager.get_page(right_no)?);
            if right.entries.len() > Self::MIN_ENTRIES {
                return self.borrow_from_right_leaf(parent_no, node_no, right_no);
            }
        }
        if let Some(left_no) = left_sib {
            let left = LeafNode::decode(self.pager.get_page(left_no)?);
            if left.entries.len() > Self::MIN_ENTRIES {
                return self.borrow_from_left_leaf(parent_no, left_no, node_no);
            }
        }
        if let Some(right_no) = right_sib {
            return self.merge_leaves(path, parent_no, node_no, right_no);
        }
        if let Some(left_no) = left_sib {
            return self.merge_leaves(path, parent_no, left_no, node_no);
        }
        Ok(())
    }

    fn borrow_from_right_leaf(&mut self, parent_no: u32, node_no: u32, right_no: u32) -> Result<(), BTreeError> {
        let mut node = LeafNode::decode(self.pager.get_page(node_no)?);
        let mut right = LeafNode::decode(self.pager.get_page(right_no)?);
        let moved = right.entries.remove(0);
        let new_separator = right.entries[0].key.clone();
        node.entries.push(moved);
        node.encode(self.pager.get_page_mut(node_no)?);
        right.encode(self.pager.get_page_mut(right_no)?);
        self.update_separator(parent_no, node_no, new_separator)
    }

    fn borrow_from_left_leaf(&mut self, parent_no: u32, left_no: u32, node_no: u32) -> Result<(), BTreeError> {
        let mut left = LeafNode::decode(self.pager.get_page(left_no)?);
        let mut node = LeafNode::decode(self.pager.get_page(node_no)?);
        let moved = left.entries.pop().unwrap();
        let new_separator = moved.key.clone();
        node.entries.insert(0, moved);
        left.encode(self.pager.get_page_mut(left_no)?);
        node.encode(self.pager.get_page_mut(node_no)?);
        self.update_separator(parent_no, left_no, new_separator)
    }

    fn merge_leaves(&mut self, path: Vec<u32>, parent_no: u32, left_no: u32, right_no: u32) -> Result<(), BTreeError> {
        let mut left = LeafNode::decode(self.pager.get_page(left_no)?);
        let right = LeafNode::decode(self.pager.get_page(right_no)?);
        left.entries.extend(right.entries);
        left.next_leaf = right.next_leaf;
        left.encode(self.pager.get_page_mut(left_no)?);
        self.pager.free_page(right_no)?;

        let page = self.pager.get_page(parent_no)?;
        let mut parent = InternalNode::decode(page);
        let i = parent.entries.iter().position(|e| e.left_child == left_no).unwrap();
        parent.entries.remove(i);
        if parent.rightmost_child == right_no {
            parent.rightmost_child = left_no;
        } else {
            let j = parent.entries.iter().position(|e| e.left_child == right_no).unwrap();
            parent.entries[j].left_child = left_no;
        }
        parent.encode(self.pager.get_page_mut(parent_no)?);

        self.rebalance_internal(path, parent_no)
    }

    fn rebalance_internal(&mut self, _path: Vec<u32>, _node_no: u32) -> Result<(), BTreeError> {
        unimplemented!("implemented in Task 15")
    }
```

- [ ] **Step 4: Run tests to verify basic delete passes; underflow test hits the stub**

Run: `cargo test btree::tree::tests::delete_removes_key_from_single_leaf btree::tree::tests::delete_missing_key_returns_false`
Expected: PASS (2 tests)

Run: `cargo test btree::tree::tests::delete_causing_leaf_underflow_borrows_or_merges_and_stays_valid`
Expected: FAIL with `unimplemented` — the large-key insert/delete sequence forces enough leaf splits and merges that an internal node underflows and reaches `rebalance_internal`. This is expected — completed in Task 15, where this same test (unmodified) becomes the primary proof that internal rebalance works end-to-end.

- [ ] **Step 5: Commit the leaf-level delete path**

```bash
git add src/btree/tree.rs
git commit -m "Add BTree delete with leaf-level borrow and merge"
```

---

## Task 15: BTree delete — internal rebalance and root collapse

**Files:**
- Modify: `src/btree/tree.rs`

**Interfaces:**
- Consumes: everything from Task 14.
- Produces: working `rebalance_internal`, completing `delete` for trees of any depth, including collapsing the root when it's left with a single child.

- [ ] **Step 1: Confirm the still-failing test from Task 14**

Run: `cargo test btree::tree::tests::delete_causing_leaf_underflow_borrows_or_merges_and_stays_valid`
Expected: FAIL with `unimplemented`.

- [ ] **Step 2: Implement `rebalance_internal` and its borrow/merge helpers**

Replace the `rebalance_internal` stub in `src/btree/tree.rs` and add its helpers:

```rust
    fn rebalance_internal(&mut self, mut path: Vec<u32>, node_no: u32) -> Result<(), BTreeError> {
        if node_no == self.root {
            let node = InternalNode::decode(self.pager.get_page(node_no)?);
            if node.entries.is_empty() {
                self.root = node.rightmost_child;
            }
            return Ok(());
        }
        let node = InternalNode::decode(self.pager.get_page(node_no)?);
        if node.entries.len() >= Self::MIN_ENTRIES {
            return Ok(());
        }
        let parent_no = path.pop().unwrap();
        let (left_sib, right_sib) = self.find_siblings(parent_no, node_no)?;

        if let Some(right_no) = right_sib {
            let right = InternalNode::decode(self.pager.get_page(right_no)?);
            if right.entries.len() > Self::MIN_ENTRIES {
                return self.borrow_from_right_internal(parent_no, node_no, right_no);
            }
        }
        if let Some(left_no) = left_sib {
            let left = InternalNode::decode(self.pager.get_page(left_no)?);
            if left.entries.len() > Self::MIN_ENTRIES {
                return self.borrow_from_left_internal(parent_no, left_no, node_no);
            }
        }
        if let Some(right_no) = right_sib {
            return self.merge_internal(path, parent_no, node_no, right_no);
        }
        if let Some(left_no) = left_sib {
            return self.merge_internal(path, parent_no, left_no, node_no);
        }
        Ok(())
    }

    fn borrow_from_right_internal(&mut self, parent_no: u32, node_no: u32, right_no: u32) -> Result<(), BTreeError> {
        let mut node = InternalNode::decode(self.pager.get_page(node_no)?);
        let mut right = InternalNode::decode(self.pager.get_page(right_no)?);
        let page = self.pager.get_page(parent_no)?;
        let mut parent = InternalNode::decode(page);
        let sep_idx = parent.entries.iter().position(|e| e.left_child == node_no).unwrap();
        let sep_key = parent.entries[sep_idx].key.clone();

        let moved = right.entries.remove(0);
        node.entries.push(InternalEntry { key: sep_key, left_child: node.rightmost_child });
        node.rightmost_child = moved.left_child;
        parent.entries[sep_idx].key = moved.key;

        node.encode(self.pager.get_page_mut(node_no)?);
        right.encode(self.pager.get_page_mut(right_no)?);
        parent.encode(self.pager.get_page_mut(parent_no)?);
        Ok(())
    }

    fn borrow_from_left_internal(&mut self, parent_no: u32, left_no: u32, node_no: u32) -> Result<(), BTreeError> {
        let mut left = InternalNode::decode(self.pager.get_page(left_no)?);
        let mut node = InternalNode::decode(self.pager.get_page(node_no)?);
        let page = self.pager.get_page(parent_no)?;
        let mut parent = InternalNode::decode(page);
        let sep_idx = parent.entries.iter().position(|e| e.left_child == left_no).unwrap();
        let sep_key = parent.entries[sep_idx].key.clone();

        let moved_child = left.rightmost_child;
        let last_entry = left.entries.pop().unwrap();
        left.rightmost_child = last_entry.left_child;

        node.entries.insert(0, InternalEntry { key: sep_key, left_child: moved_child });
        parent.entries[sep_idx].key = last_entry.key;

        left.encode(self.pager.get_page_mut(left_no)?);
        node.encode(self.pager.get_page_mut(node_no)?);
        parent.encode(self.pager.get_page_mut(parent_no)?);
        Ok(())
    }

    fn merge_internal(&mut self, path: Vec<u32>, parent_no: u32, left_no: u32, right_no: u32) -> Result<(), BTreeError> {
        let mut left = InternalNode::decode(self.pager.get_page(left_no)?);
        let right = InternalNode::decode(self.pager.get_page(right_no)?);
        let page = self.pager.get_page(parent_no)?;
        let mut parent = InternalNode::decode(page);
        let sep_idx = parent.entries.iter().position(|e| e.left_child == left_no).unwrap();
        let sep_key = parent.entries[sep_idx].key.clone();

        left.entries.push(InternalEntry { key: sep_key, left_child: left.rightmost_child });
        left.entries.extend(right.entries);
        left.rightmost_child = right.rightmost_child;
        left.encode(self.pager.get_page_mut(left_no)?);
        self.pager.free_page(right_no)?;

        parent.entries.remove(sep_idx);
        if parent.rightmost_child == right_no {
            parent.rightmost_child = left_no;
        } else {
            let j = parent.entries.iter().position(|e| e.left_child == right_no).unwrap();
            parent.entries[j].left_child = left_no;
        }
        parent.encode(self.pager.get_page_mut(parent_no)?);

        self.rebalance_internal(path, parent_no)
    }
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test btree::tree::tests`
Expected: PASS (all tests through Task 15, including the strengthened underflow test)

- [ ] **Step 4: Commit**

```bash
git add src/btree/tree.rs
git commit -m "Add internal node borrow/merge and root collapse to BTree delete"
```

---

## Task 16: BTree delete property test

**Files:**
- Modify: `src/btree/tree.rs`

**Interfaces:**
- Consumes: everything from Task 15.
- Produces: no new public interface — a property test proving delete is correct under random insert/delete sequences.

- [ ] **Step 1: Write the property test**

Add to the `tests` module in `src/btree/tree.rs`:

```rust
    // Zero-padded numeric prefix preserves ordering, and the ~700-byte tail
    // caps a leaf at ~5 entries so splits/merges/borrows/root-collapse are
    // actually reachable within a couple hundred ops -- same technique as
    // Tasks 10/14/15's fixed tests. Plain 4-byte keys never exceed ~11
    // bytes/entry, so even 200+ of them fit in a single 4096-byte root leaf
    // and never split at all: the first version of this property test used
    // k.to_be_bytes() directly and passed, but never exercised any of the
    // split/merge/borrow/root-collapse code Tasks 9-15 built. Task 16's
    // reviewer worked out the exact byte-size bound proving this was
    // structurally impossible, not just something that happened not to occur.
    fn padded_key(k: u32) -> Vec<u8> {
        format!("{k:06}{}", "x".repeat(700)).into_bytes()
    }

    fn unpad_key(key: &[u8]) -> u32 {
        std::str::from_utf8(&key[0..6]).unwrap().parse().unwrap()
    }

    proptest! {
        // Each case runs up to 400 insert/delete ops with a full check_invariants()
        // tree walk after every single one -- genuinely thorough, but at the
        // default 256 cases this test alone takes roughly two minutes, and cargo
        // test (unscoped) still runs several more times across the rest of this
        // project's plan. 48 cases keeps real random coverage (still far more
        // than any single hand-written scenario) while cutting this to a
        // manageable runtime.
        #![proptest_config(ProptestConfig::with_cases(48))]
        #[test]
        fn insert_delete_sequence_stays_consistent(
            ops in pvec((0u32..200, any::<bool>()), 1..400)
        ) {
            let (mut pager, root) = empty_tree();
            let mut bt = BTree::new(&mut pager, root);
            let mut model = std::collections::BTreeSet::new();

            for (k, is_insert) in ops {
                if is_insert {
                    if bt.insert(&padded_key(k), b"v").is_ok() {
                        model.insert(k);
                    }
                } else if bt.delete(&padded_key(k)).unwrap() {
                    model.remove(&k);
                }
                bt.check_invariants().unwrap();
            }

            for k in &model {
                prop_assert_eq!(bt.search(&padded_key(*k)).unwrap(), Some(b"v".to_vec()));
            }
            let mut cursor = bt.cursor_start().unwrap();
            let mut scanned = Vec::new();
            while let Some((key, _)) = cursor.next(&mut pager).unwrap() {
                scanned.push(unpad_key(&key));
            }
            let expected: Vec<u32> = model.iter().copied().collect();
            prop_assert_eq!(scanned, expected);
        }
    }
```

Note: this closure captures `pager` and `bt` both — since `bt` borrows `pager` mutably, the line `cursor.next(&mut pager)` after `bt.cursor_start()` requires `bt`'s borrow to have ended. Since `cursor_start` returns an owned `Cursor` and `bt` itself is not used again after that call in this test body, the borrow checker accepts this as written. If a compile error surfaces here, wrap the `bt.cursor_start()` call in its own block as done in Task 13's tests.

- [ ] **Step 2: Run the property test**

Run: `cargo test btree::tree::tests::insert_delete_sequence_stays_consistent`
Expected: PASS. If a borrow-checker error appears per the note above, apply the block-scoping fix and re-run.

- [ ] **Step 3: Run the entire BTree test suite**

Run: `cargo test btree::`
Expected: PASS — this is the last BTree task; all of Tasks 8–16 should be green together.

- [ ] **Step 4: Commit**

```bash
git add src/btree/tree.rs
git commit -m "Add insert/delete property test proving BTree consistency"
```

---

## Task 17: Catalog record encoding

**Files:**
- Create: `src/catalog/record.rs`

**Interfaces:**
- Consumes: `schema::{Column, TableSchema, IndexSchema}`, `value::ColumnType`.
- Produces: `record::{encode_table_record, decode_table_record, encode_index_record, decode_index_record, record_kind}`. `record_kind(&[u8]) -> u8` returns `1` for a table record, `2` for an index record — used by Task 18's listing functions without fully decoding every record.

- [ ] **Step 1: Write the failing test**

Write `src/catalog/record.rs`:

```rust
use crate::types::schema::{Column, IndexSchema, TableSchema};
use crate::types::value::ColumnType;

const KIND_TABLE: u8 = 1;
const KIND_INDEX: u8 = 2;
const TYPE_INTEGER: u8 = 1;
const TYPE_TEXT: u8 = 2;
const TYPE_BOOLEAN: u8 = 3;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_record_roundtrip() {
        let schema = TableSchema {
            name: "users".into(),
            columns: vec![
                Column { name: "id".into(), ty: ColumnType::Integer, not_null: true, is_primary_key: true },
                Column { name: "email".into(), ty: ColumnType::Text, not_null: false, is_primary_key: false },
            ],
            root_page: 7,
        };
        let encoded = encode_table_record(&schema);
        assert_eq!(record_kind(&encoded), KIND_TABLE);
        let decoded = decode_table_record(&encoded);
        assert_eq!(decoded.name, "users");
        assert_eq!(decoded.root_page, 7);
        assert_eq!(decoded.columns.len(), 2);
        assert_eq!(decoded.columns[0].name, "id");
        assert!(decoded.columns[0].is_primary_key);
        assert_eq!(decoded.columns[1].ty, ColumnType::Text);
    }

    #[test]
    fn index_record_roundtrip() {
        let schema = IndexSchema {
            name: "idx_email".into(),
            table: "users".into(),
            column: "email".into(),
            root_page: 12,
        };
        let encoded = encode_index_record(&schema);
        assert_eq!(record_kind(&encoded), KIND_INDEX);
        let decoded = decode_index_record(&encoded);
        assert_eq!(decoded.name, "idx_email");
        assert_eq!(decoded.table, "users");
        assert_eq!(decoded.column, "email");
        assert_eq!(decoded.root_page, 12);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test catalog::record::tests`
Expected: FAIL — encode/decode functions not defined.

- [ ] **Step 3: Implement**

Add above the test module:

```rust
pub fn record_kind(data: &[u8]) -> u8 {
    data[0]
}

fn write_string(out: &mut Vec<u8>, s: &str) {
    out.extend(&(s.len() as u16).to_le_bytes());
    out.extend(s.as_bytes());
}

fn read_string(data: &[u8], pos: usize) -> (String, usize) {
    let len = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap()) as usize;
    let s = String::from_utf8(data[pos + 2..pos + 2 + len].to_vec()).unwrap();
    (s, pos + 2 + len)
}

fn type_tag(ty: &ColumnType) -> u8 {
    match ty {
        ColumnType::Integer => TYPE_INTEGER,
        ColumnType::Text => TYPE_TEXT,
        ColumnType::Boolean => TYPE_BOOLEAN,
    }
}

fn type_from_tag(tag: u8) -> ColumnType {
    match tag {
        TYPE_INTEGER => ColumnType::Integer,
        TYPE_TEXT => ColumnType::Text,
        TYPE_BOOLEAN => ColumnType::Boolean,
        _ => panic!("unknown column type tag {tag}"),
    }
}

pub fn encode_table_record(schema: &TableSchema) -> Vec<u8> {
    let mut out = vec![KIND_TABLE];
    write_string(&mut out, &schema.name);
    out.extend(&schema.root_page.to_le_bytes());
    out.extend(&(schema.columns.len() as u16).to_le_bytes());
    for col in &schema.columns {
        write_string(&mut out, &col.name);
        out.push(type_tag(&col.ty));
        out.push(col.not_null as u8);
        out.push(col.is_primary_key as u8);
    }
    out
}

pub fn decode_table_record(data: &[u8]) -> TableSchema {
    assert_eq!(data[0], KIND_TABLE);
    let mut pos = 1;
    let (name, next) = read_string(data, pos);
    pos = next;
    let root_page = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap());
    pos += 4;
    let num_cols = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap()) as usize;
    pos += 2;
    let mut columns = Vec::with_capacity(num_cols);
    for _ in 0..num_cols {
        let (cname, next) = read_string(data, pos);
        pos = next;
        let ty = type_from_tag(data[pos]);
        pos += 1;
        let not_null = data[pos] != 0;
        pos += 1;
        let is_primary_key = data[pos] != 0;
        pos += 1;
        columns.push(Column { name: cname, ty, not_null, is_primary_key });
    }
    TableSchema { name, columns, root_page }
}

pub fn encode_index_record(schema: &IndexSchema) -> Vec<u8> {
    let mut out = vec![KIND_INDEX];
    write_string(&mut out, &schema.name);
    write_string(&mut out, &schema.table);
    write_string(&mut out, &schema.column);
    out.extend(&schema.root_page.to_le_bytes());
    out
}

pub fn decode_index_record(data: &[u8]) -> IndexSchema {
    assert_eq!(data[0], KIND_INDEX);
    let mut pos = 1;
    let (name, next) = read_string(data, pos);
    pos = next;
    let (table, next) = read_string(data, pos);
    pos = next;
    let (column, next) = read_string(data, pos);
    pos = next;
    let root_page = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap());
    IndexSchema { name, table, column, root_page }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test catalog::record::tests`
Expected: PASS (2 tests)

- [ ] **Step 5: Commit**

```bash
git add src/catalog/record.rs
git commit -m "Add catalog record encoding for tables and indexes"
```

---

## Task 18: Catalog — bootstrap, table/index CRUD, listing, root updates

**Files:**
- Create: `src/catalog/mod.rs` (already declared as `pub mod record;` in Task 1's `src/catalog.rs`; add the `Catalog` struct directly to `src/catalog.rs` instead — see Step 1)

**Interfaces:**
- Consumes: `btree::tree::BTree`, `btree::node::{LeafNode, InternalNode}`, `Pager`, `record::*`, `value::{Value, encode_key}`.
- Produces: `catalog::Catalog` with `bootstrap`, `create_table`, `get_table`, `update_table_root`, `drop_table`, `create_index`, `get_index`, `update_index_root`, `drop_index`, `list_tables`, `list_indexes_for_table`, all taking `&mut Pager` explicitly (the catalog does not own the pager).

- [ ] **Step 1: Move `Catalog` into `src/catalog.rs` directly**

Since the catalog needs no further submodules beyond `record`, put the `Catalog` struct in `src/catalog.rs` itself rather than adding a `catalog/mod.rs`. Edit `src/catalog.rs` (currently just `pub mod record;` from Task 1) to also declare the struct in the same file, appending below the `pub mod record;` line.

- [ ] **Step 2: Write the failing test**

Add to `src/catalog.rs`:

```rust
pub mod record;

use crate::btree::node::{InternalNode, LeafNode};
use crate::btree::tree::BTree;
use crate::error::{BTreeError, DbError, PlanError, StorageError};
use crate::storage::page::{PAGE_TYPE_INTERNAL, PAGE_TYPE_LEAF};
use crate::storage::pager::Pager;
use crate::types::schema::{IndexSchema, TableSchema};
use crate::types::value::{encode_key, Value};
use record::*;

pub struct Catalog {
    root: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::schema::Column;
    use crate::types::value::ColumnType;
    use tempfile::NamedTempFile;

    fn sample_schema(name: &str) -> TableSchema {
        TableSchema {
            name: name.into(),
            columns: vec![
                Column { name: "id".into(), ty: ColumnType::Integer, not_null: true, is_primary_key: true },
            ],
            root_page: 0, // filled in by the caller after allocating a table root
        }
    }

    #[test]
    fn create_and_get_table_roundtrips() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let mut catalog = Catalog::bootstrap(&mut pager).unwrap();

        let table_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(table_root).unwrap());
        let mut schema = sample_schema("users");
        schema.root_page = table_root;
        catalog.create_table(&mut pager, &schema).unwrap();

        let fetched = catalog.get_table(&mut pager, "users").unwrap().unwrap();
        assert_eq!(fetched.name, "users");
        assert_eq!(fetched.root_page, table_root);
        assert!(catalog.get_table(&mut pager, "missing").unwrap().is_none());
    }

    #[test]
    fn create_duplicate_table_errors() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let mut catalog = Catalog::bootstrap(&mut pager).unwrap();
        let mut schema = sample_schema("users");
        schema.root_page = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(schema.root_page).unwrap());
        catalog.create_table(&mut pager, &schema).unwrap();
        let err = catalog.create_table(&mut pager, &schema).unwrap_err();
        assert!(matches!(err, DbError::Plan(PlanError::TableAlreadyExists(_))));
    }

    #[test]
    fn drop_table_removes_it_and_frees_pages() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let mut catalog = Catalog::bootstrap(&mut pager).unwrap();
        let mut schema = sample_schema("users");
        schema.root_page = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(schema.root_page).unwrap());
        catalog.create_table(&mut pager, &schema).unwrap();

        catalog.drop_table(&mut pager, "users").unwrap();
        assert!(catalog.get_table(&mut pager, "users").unwrap().is_none());
    }

    #[test]
    fn list_tables_returns_created_names() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let mut catalog = Catalog::bootstrap(&mut pager).unwrap();
        for name in ["a", "b", "c"] {
            let mut schema = sample_schema(name);
            schema.root_page = pager.allocate_page().unwrap();
            LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(schema.root_page).unwrap());
            catalog.create_table(&mut pager, &schema).unwrap();
        }
        let mut names = catalog.list_tables(&mut pager).unwrap();
        names.sort();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[test]
    fn create_index_get_and_drop() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let mut catalog = Catalog::bootstrap(&mut pager).unwrap();
        let idx_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(idx_root).unwrap());
        let idx = IndexSchema { name: "idx_id".into(), table: "users".into(), column: "id".into(), root_page: idx_root };
        catalog.create_index(&mut pager, &idx).unwrap();

        let fetched = catalog.get_index(&mut pager, "idx_id").unwrap().unwrap();
        assert_eq!(fetched.table, "users");

        let for_table = catalog.list_indexes_for_table(&mut pager, "users").unwrap();
        assert_eq!(for_table.len(), 1);

        catalog.drop_index(&mut pager, "idx_id").unwrap();
        assert!(catalog.get_index(&mut pager, "idx_id").unwrap().is_none());
    }

    #[test]
    fn update_table_root_persists_new_root() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let mut catalog = Catalog::bootstrap(&mut pager).unwrap();
        let mut schema = sample_schema("users");
        schema.root_page = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(schema.root_page).unwrap());
        catalog.create_table(&mut pager, &schema).unwrap();

        let new_root = pager.allocate_page().unwrap();
        catalog.update_table_root(&mut pager, "users", new_root).unwrap();
        let fetched = catalog.get_table(&mut pager, "users").unwrap().unwrap();
        assert_eq!(fetched.root_page, new_root);
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test catalog::tests`
Expected: FAIL — `Catalog::bootstrap` and friends not defined.

- [ ] **Step 4: Implement `Catalog`**

Add above the test module in `src/catalog.rs`:

```rust
impl Catalog {
    pub fn bootstrap(pager: &mut Pager) -> Result<Catalog, DbError> {
        if pager.catalog_root() == 0 {
            let root_page = pager.allocate_page()?;
            LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(root_page)?);
            pager.set_catalog_root(root_page)?;
        }
        Ok(Catalog { root: pager.catalog_root() })
    }

    fn table_key(name: &str) -> Vec<u8> {
        encode_key(&Value::Text(format!("table:{name}")))
    }

    fn index_key(name: &str) -> Vec<u8> {
        encode_key(&Value::Text(format!("index:{name}")))
    }

    pub fn create_table(&mut self, pager: &mut Pager, schema: &TableSchema) -> Result<(), DbError> {
        let key = Self::table_key(&schema.name);
        let mut bt = BTree::new(pager, self.root);
        bt.insert(&key, &encode_table_record(schema)).map_err(|e| match e {
            BTreeError::DuplicateKey => DbError::Plan(PlanError::TableAlreadyExists(schema.name.clone())),
            other => DbError::BTree(other),
        })?;
        self.root = bt.root();
        pager.set_catalog_root(self.root)?;
        Ok(())
    }

    pub fn get_table(&mut self, pager: &mut Pager, name: &str) -> Result<Option<TableSchema>, DbError> {
        let key = Self::table_key(name);
        let mut bt = BTree::new(pager, self.root);
        Ok(bt.search(&key)?.map(|p| decode_table_record(&p)))
    }

    pub fn update_table_root(&mut self, pager: &mut Pager, name: &str, new_root: u32) -> Result<(), DbError> {
        let mut schema = self
            .get_table(pager, name)?
            .ok_or_else(|| PlanError::NoSuchTable(name.to_string()))?;
        schema.root_page = new_root;
        let key = Self::table_key(name);
        let mut bt = BTree::new(pager, self.root);
        bt.delete(&key)?;
        bt.insert(&key, &encode_table_record(&schema))?;
        self.root = bt.root();
        pager.set_catalog_root(self.root)?;
        Ok(())
    }

    pub fn drop_table(&mut self, pager: &mut Pager, name: &str) -> Result<(), DbError> {
        let schema = self
            .get_table(pager, name)?
            .ok_or_else(|| PlanError::NoSuchTable(name.to_string()))?;
        for idx in self.list_indexes_for_table(pager, name)? {
            self.drop_index(pager, &idx.name)?;
        }
        walk_and_free(pager, schema.root_page)?;
        let key = Self::table_key(name);
        let mut bt = BTree::new(pager, self.root);
        bt.delete(&key)?;
        self.root = bt.root();
        pager.set_catalog_root(self.root)?;
        Ok(())
    }

    pub fn create_index(&mut self, pager: &mut Pager, schema: &IndexSchema) -> Result<(), DbError> {
        let key = Self::index_key(&schema.name);
        let mut bt = BTree::new(pager, self.root);
        bt.insert(&key, &encode_index_record(schema)).map_err(|e| match e {
            BTreeError::DuplicateKey => DbError::Plan(PlanError::IndexAlreadyExists(schema.name.clone())),
            other => DbError::BTree(other),
        })?;
        self.root = bt.root();
        pager.set_catalog_root(self.root)?;
        Ok(())
    }

    pub fn get_index(&mut self, pager: &mut Pager, name: &str) -> Result<Option<IndexSchema>, DbError> {
        let key = Self::index_key(name);
        let mut bt = BTree::new(pager, self.root);
        Ok(bt.search(&key)?.map(|p| decode_index_record(&p)))
    }

    pub fn update_index_root(&mut self, pager: &mut Pager, name: &str, new_root: u32) -> Result<(), DbError> {
        let mut schema = self
            .get_index(pager, name)?
            .ok_or_else(|| PlanError::NoSuchIndex(name.to_string()))?;
        schema.root_page = new_root;
        let key = Self::index_key(name);
        let mut bt = BTree::new(pager, self.root);
        bt.delete(&key)?;
        bt.insert(&key, &encode_index_record(&schema))?;
        self.root = bt.root();
        pager.set_catalog_root(self.root)?;
        Ok(())
    }

    pub fn drop_index(&mut self, pager: &mut Pager, name: &str) -> Result<(), DbError> {
        let schema = self
            .get_index(pager, name)?
            .ok_or_else(|| PlanError::NoSuchIndex(name.to_string()))?;
        walk_and_free(pager, schema.root_page)?;
        let key = Self::index_key(name);
        let mut bt = BTree::new(pager, self.root);
        bt.delete(&key)?;
        self.root = bt.root();
        pager.set_catalog_root(self.root)?;
        Ok(())
    }

    pub fn list_tables(&mut self, pager: &mut Pager) -> Result<Vec<String>, DbError> {
        let mut cursor = {
            let mut bt = BTree::new(pager, self.root);
            bt.cursor_start()?
        };
        let mut names = Vec::new();
        while let Some((_, payload)) = cursor.next(pager)? {
            if record_kind(&payload) == 1 {
                names.push(decode_table_record(&payload).name);
            }
        }
        Ok(names)
    }

    pub fn list_indexes_for_table(&mut self, pager: &mut Pager, table: &str) -> Result<Vec<IndexSchema>, DbError> {
        let mut cursor = {
            let mut bt = BTree::new(pager, self.root);
            bt.cursor_start()?
        };
        let mut result = Vec::new();
        while let Some((_, payload)) = cursor.next(pager)? {
            if record_kind(&payload) == 2 {
                let idx = decode_index_record(&payload);
                if idx.table == table {
                    result.push(idx);
                }
            }
        }
        Ok(result)
    }
}

fn walk_and_free(pager: &mut Pager, page_no: u32) -> Result<(), StorageError> {
    let page_type = pager.get_page(page_no)?.page_type();
    if page_type == PAGE_TYPE_INTERNAL {
        let node = InternalNode::decode(pager.get_page(page_no)?);
        let children: Vec<u32> = node
            .entries
            .iter()
            .map(|e| e.left_child)
            .chain(std::iter::once(node.rightmost_child))
            .collect();
        for c in children {
            walk_and_free(pager, c)?;
        }
    } else if page_type != PAGE_TYPE_LEAF {
        return Ok(());
    }
    pager.free_page(page_no)
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test catalog::tests`
Expected: PASS (6 tests)

- [ ] **Step 6: Commit**

```bash
git add src/catalog.rs
git commit -m "Add catalog with table/index CRUD, listing, and root-update bookkeeping"
```

---

## Task 19: SQL lexer

**Files:**
- Create: `src/sql/token.rs`
- Create: `src/sql/lexer.rs`

**Interfaces:**
- Consumes: `ParseError`.
- Produces: `token::{Token, SpannedToken}`; `lexer::tokenize(&str) -> Result<Vec<SpannedToken>, ParseError>`.

- [ ] **Step 1: Write `src/sql/token.rs`**

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Create, Table, Drop, Index, On, Insert, Into, Values, Select, From, Where,
    Update, Set, Delete, Order, By, Asc, Desc, Limit, Not, Null, Primary, Key,
    And, Or, Is,
    KwInteger, KwText, KwBoolean,
    Identifier(String),
    IntLiteral(i64),
    StringLiteral(String),
    True, False,
    Eq, NotEq, Lt, LtEq, Gt, GtEq,
    LParen, RParen, Comma, Star, Semicolon,
    Eof,
}

#[derive(Debug, Clone)]
pub struct SpannedToken {
    pub token: Token,
    pub offset: usize,
}
```

- [ ] **Step 2: Write the failing test**

Write `src/sql/lexer.rs`:

```rust
use crate::error::ParseError;
use crate::sql::token::{SpannedToken, Token};

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<Token> {
        tokenize(src).unwrap().into_iter().map(|t| t.token).collect()
    }

    #[test]
    fn tokenizes_create_table() {
        let tokens = kinds("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)");
        assert_eq!(
            tokens,
            vec![
                Token::Create, Token::Table, Token::Identifier("users".into()), Token::LParen,
                Token::Identifier("id".into()), Token::KwInteger, Token::Primary, Token::Key, Token::Comma,
                Token::Identifier("name".into()), Token::KwText, Token::Not, Token::Null,
                Token::RParen, Token::Eof,
            ]
        );
    }

    #[test]
    fn tokenizes_operators() {
        assert_eq!(kinds("<= >= <> != < > ="), vec![
            Token::LtEq, Token::GtEq, Token::NotEq, Token::NotEq, Token::Lt, Token::Gt, Token::Eq, Token::Eof
        ]);
    }

    #[test]
    fn tokenizes_string_literal_with_escaped_quote() {
        assert_eq!(kinds("'it''s'"), vec![Token::StringLiteral("it's".into()), Token::Eof]);
    }

    #[test]
    fn tokenizes_keywords_case_insensitively() {
        assert_eq!(kinds("select FROM Where"), vec![Token::Select, Token::From, Token::Where, Token::Eof]);
    }

    #[test]
    fn reports_offset_on_unterminated_string() {
        let err = tokenize("'abc").unwrap_err();
        match err {
            ParseError::Syntax { offset, .. } => assert_eq!(offset, 0),
        }
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test sql::lexer::tests`
Expected: FAIL — `tokenize` not defined.

- [ ] **Step 4: Implement `tokenize`**

Add above the test module:

```rust
pub fn tokenize(src: &str) -> Result<Vec<SpannedToken>, ParseError> {
    // Iterate over decoded `char`s (via `char_indices`), not raw bytes: casting an
    // arbitrary byte to `char` misinterprets UTF-8 continuation bytes as Latin-1
    // code points, some of which look "alphabetic" to Rust's char classification --
    // corrupting string-literal content and risking a "not a char boundary" panic
    // on any non-ASCII input (e.g. a TEXT literal like 'café'). `char_indices()`
    // still reports byte offsets, so `SpannedToken.offset` is unaffected.
    let mut chars = src.char_indices().peekable();
    let mut tokens = Vec::new();

    while let Some(&(start, c)) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        match c {
            '(' => { chars.next(); tokens.push(SpannedToken { token: Token::LParen, offset: start }); }
            ')' => { chars.next(); tokens.push(SpannedToken { token: Token::RParen, offset: start }); }
            ',' => { chars.next(); tokens.push(SpannedToken { token: Token::Comma, offset: start }); }
            '*' => { chars.next(); tokens.push(SpannedToken { token: Token::Star, offset: start }); }
            ';' => { chars.next(); tokens.push(SpannedToken { token: Token::Semicolon, offset: start }); }
            '=' => { chars.next(); tokens.push(SpannedToken { token: Token::Eq, offset: start }); }
            '<' => {
                chars.next();
                match chars.peek() {
                    Some(&(_, '=')) => {
                        chars.next();
                        tokens.push(SpannedToken { token: Token::LtEq, offset: start });
                    }
                    Some(&(_, '>')) => {
                        chars.next();
                        tokens.push(SpannedToken { token: Token::NotEq, offset: start });
                    }
                    _ => tokens.push(SpannedToken { token: Token::Lt, offset: start }),
                }
            }
            '>' => {
                chars.next();
                match chars.peek() {
                    Some(&(_, '=')) => {
                        chars.next();
                        tokens.push(SpannedToken { token: Token::GtEq, offset: start });
                    }
                    _ => tokens.push(SpannedToken { token: Token::Gt, offset: start }),
                }
            }
            '!' => {
                chars.next();
                match chars.peek() {
                    Some(&(_, '=')) => {
                        chars.next();
                        tokens.push(SpannedToken { token: Token::NotEq, offset: start });
                    }
                    _ => return Err(ParseError::Syntax { offset: start, message: "unexpected '!'".into() }),
                }
            }
            '\'' => {
                chars.next();
                let mut s = String::new();
                loop {
                    match chars.next() {
                        None => {
                            return Err(ParseError::Syntax {
                                offset: start,
                                message: "unterminated string literal".into(),
                            })
                        }
                        Some((_, '\'')) => {
                            if let Some(&(_, '\'')) = chars.peek() {
                                chars.next();
                                s.push('\'');
                            } else {
                                break;
                            }
                        }
                        Some((_, ch)) => s.push(ch),
                    }
                }
                tokens.push(SpannedToken { token: Token::StringLiteral(s), offset: start });
            }
            c if c.is_ascii_digit() => {
                let mut end = start + c.len_utf8();
                chars.next();
                while let Some(&(p, c2)) = chars.peek() {
                    if c2.is_ascii_digit() {
                        end = p + c2.len_utf8();
                        chars.next();
                    } else {
                        break;
                    }
                }
                let text = &src[start..end];
                let n: i64 = text
                    .parse()
                    .map_err(|_| ParseError::Syntax { offset: start, message: "invalid integer literal".into() })?;
                tokens.push(SpannedToken { token: Token::IntLiteral(n), offset: start });
            }
            c if c.is_alphabetic() || c == '_' => {
                let mut end = start + c.len_utf8();
                chars.next();
                while let Some(&(p, c2)) = chars.peek() {
                    if c2.is_alphanumeric() || c2 == '_' {
                        end = p + c2.len_utf8();
                        chars.next();
                    } else {
                        break;
                    }
                }
                let text = &src[start..end];
                tokens.push(SpannedToken { token: keyword_or_identifier(text), offset: start });
            }
            _ => return Err(ParseError::Syntax { offset: start, message: format!("unexpected character '{c}'") }),
        }
    }
    tokens.push(SpannedToken { token: Token::Eof, offset: src.len() });
    Ok(tokens)
}

fn keyword_or_identifier(text: &str) -> Token {
    match text.to_uppercase().as_str() {
        "CREATE" => Token::Create,
        "TABLE" => Token::Table,
        "DROP" => Token::Drop,
        "INDEX" => Token::Index,
        "ON" => Token::On,
        "INSERT" => Token::Insert,
        "INTO" => Token::Into,
        "VALUES" => Token::Values,
        "SELECT" => Token::Select,
        "FROM" => Token::From,
        "WHERE" => Token::Where,
        "UPDATE" => Token::Update,
        "SET" => Token::Set,
        "DELETE" => Token::Delete,
        "ORDER" => Token::Order,
        "BY" => Token::By,
        "ASC" => Token::Asc,
        "DESC" => Token::Desc,
        "LIMIT" => Token::Limit,
        "NOT" => Token::Not,
        "NULL" => Token::Null,
        "PRIMARY" => Token::Primary,
        "KEY" => Token::Key,
        "AND" => Token::And,
        "OR" => Token::Or,
        "IS" => Token::Is,
        "INTEGER" => Token::KwInteger,
        "TEXT" => Token::KwText,
        "BOOLEAN" => Token::KwBoolean,
        "TRUE" => Token::True,
        "FALSE" => Token::False,
        _ => Token::Identifier(text.to_string()),
    }
}
```

- [ ] **Step 5: Add non-ASCII UTF-8 coverage**

The lexer iterates over decoded `char`s (not raw bytes) specifically so it handles multi-byte UTF-8 correctly — TEXT values are UTF-8 per the row/key encoding, so a string literal or identifier containing non-ASCII characters is ordinary input, not an edge case. Add tests proving this, to the `tests` module:

```rust
    #[test]
    fn tokenizes_non_ascii_utf8_string_literal_without_corruption_or_panic() {
        assert_eq!(kinds("'café €5'"), vec![Token::StringLiteral("café €5".into()), Token::Eof]);
    }

    #[test]
    fn tokenizes_non_ascii_identifier_and_reports_correct_offsets() {
        // Offsets are byte positions (matching how the rest of the engine slices
        // &str), not char counts, so a non-ASCII identifier must not desynchronize
        // the offsets of tokens that follow it.
        let tokens = tokenize("café = 1").unwrap();
        let offsets: Vec<usize> = tokens.iter().map(|t| t.offset).collect();
        assert_eq!(
            tokens.iter().map(|t| t.token.clone()).collect::<Vec<_>>(),
            vec![Token::Identifier("café".into()), Token::Eq, Token::IntLiteral(1), Token::Eof]
        );
        // "café" is 5 bytes (c=1,a=1,f=1,é=2), so '=' starts at byte offset 6 (after
        // the trailing space), not char-index 5.
        assert_eq!(offsets, vec![0, 6, 8, 9]);
    }
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test sql::lexer::tests`
Expected: PASS (7 tests)

- [ ] **Step 7: Commit**

```bash
git add src/sql/token.rs src/sql/lexer.rs
git commit -m "Add SQL lexer with case-insensitive keywords and offset-tracked tokens"
```

---

## Task 20: AST types and parser scaffold; CREATE TABLE / DROP TABLE

**Files:**
- Create: `src/sql/ast.rs`
- Create: `src/sql/parser.rs`

**Interfaces:**
- Consumes: `token::{Token, SpannedToken}`, `lexer::tokenize`, `value::ColumnType`.
- Produces: `ast::{Statement, ColumnDef, SelectColumns, Expr, BinOp}`. `parser::{Parser, parse}` where `parse(sql: &str) -> Result<Statement, ParseError>` is the public entry point every later parser task and the engine use.

- [ ] **Step 1: Write `src/sql/ast.rs`**

```rust
use crate::types::value::ColumnType;

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    CreateTable { name: String, columns: Vec<ColumnDef> },
    DropTable { name: String },
    CreateIndex { name: String, table: String, column: String },
    DropIndex { name: String },
    Insert { table: String, columns: Option<Vec<String>>, rows: Vec<Vec<Expr>> },
    Select {
        columns: SelectColumns,
        table: String,
        where_clause: Option<Expr>,
        order_by: Option<(String, bool)>,
        limit: Option<i64>,
    },
    Update { table: String, assignments: Vec<(String, Expr)>, where_clause: Option<Expr> },
    Delete { table: String, where_clause: Option<Expr> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum SelectColumns {
    All,
    List(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColumnDef {
    pub name: String,
    pub ty: ColumnType,
    pub not_null: bool,
    pub primary_key: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Column(String),
    IntLiteral(i64),
    StringLiteral(String),
    BoolLiteral(bool),
    Null,
    BinaryOp { op: BinOp, left: Box<Expr>, right: Box<Expr> },
    IsNull { expr: Box<Expr>, negated: bool },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    And, Or, Eq, NotEq, Lt, LtEq, Gt, GtEq,
}
```

- [ ] **Step 2: Write the failing test**

Write `src/sql/parser.rs`:

```rust
use crate::error::ParseError;
use crate::sql::ast::*;
use crate::sql::lexer::tokenize;
use crate::sql::token::{SpannedToken, Token};
use crate::types::value::ColumnType;

pub struct Parser {
    tokens: Vec<SpannedToken>,
    pos: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_create_table() {
        let stmt = parse("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)").unwrap();
        assert_eq!(
            stmt,
            Statement::CreateTable {
                name: "users".into(),
                columns: vec![
                    ColumnDef { name: "id".into(), ty: ColumnType::Integer, not_null: false, primary_key: true },
                    ColumnDef { name: "name".into(), ty: ColumnType::Text, not_null: true, primary_key: false },
                ],
            }
        );
    }

    #[test]
    fn parses_drop_table() {
        assert_eq!(parse("DROP TABLE users").unwrap(), Statement::DropTable { name: "users".into() });
    }

    #[test]
    fn reports_syntax_error_with_offset() {
        let err = parse("CREATE TABLE").unwrap_err();
        assert!(matches!(err, ParseError::Syntax { .. }));
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test sql::parser::tests`
Expected: FAIL — `parse` not defined.

- [ ] **Step 4: Implement the parser scaffold and CREATE TABLE / DROP TABLE**

Add above the test module:

```rust
impl Parser {
    pub fn new(tokens: Vec<SpannedToken>) -> Self {
        Parser { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.pos].token
    }

    fn peek_offset(&self) -> usize {
        self.tokens[self.pos].offset
    }

    fn advance(&mut self) -> Token {
        let t = self.tokens[self.pos].token.clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn expect(&mut self, expected: &Token) -> Result<(), ParseError> {
        if std::mem::discriminant(self.peek()) == std::mem::discriminant(expected) {
            self.advance();
            Ok(())
        } else {
            Err(ParseError::Syntax {
                offset: self.peek_offset(),
                message: format!("expected {expected:?}, found {:?}", self.peek()),
            })
        }
    }

    fn expect_identifier(&mut self) -> Result<String, ParseError> {
        let offset = self.peek_offset();
        match self.advance() {
            Token::Identifier(s) => Ok(s),
            other => Err(ParseError::Syntax { offset, message: format!("expected identifier, found {other:?}") }),
        }
    }

    pub fn parse_statement(&mut self) -> Result<Statement, ParseError> {
        let stmt = match self.peek() {
            Token::Create => self.parse_create()?,
            Token::Drop => self.parse_drop()?,
            _ => return Err(ParseError::Syntax { offset: self.peek_offset(), message: "expected a statement".into() }),
        };
        if matches!(self.peek(), Token::Semicolon) {
            self.advance();
        }
        Ok(stmt)
    }

    fn parse_create(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::Create)?;
        match self.peek() {
            Token::Table => self.parse_create_table(),
            Token::Index => Err(ParseError::Syntax { offset: self.peek_offset(), message: "CREATE INDEX not yet implemented".into() }),
            _ => Err(ParseError::Syntax { offset: self.peek_offset(), message: "expected TABLE or INDEX after CREATE".into() }),
        }
    }

    fn parse_create_table(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::Table)?;
        let name = self.expect_identifier()?;
        self.expect(&Token::LParen)?;
        let mut columns = Vec::new();
        loop {
            let cname = self.expect_identifier()?;
            let ty_offset = self.peek_offset();
            let ty = match self.advance() {
                Token::KwInteger => ColumnType::Integer,
                Token::KwText => ColumnType::Text,
                Token::KwBoolean => ColumnType::Boolean,
                other => return Err(ParseError::Syntax { offset: ty_offset, message: format!("expected a type, found {other:?}") }),
            };
            let mut not_null = false;
            let mut primary_key = false;
            loop {
                match self.peek() {
                    Token::Not => { self.advance(); self.expect(&Token::Null)?; not_null = true; }
                    Token::Primary => { self.advance(); self.expect(&Token::Key)?; primary_key = true; }
                    _ => break,
                }
            }
            columns.push(ColumnDef { name: cname, ty, not_null, primary_key });
            match self.peek() {
                Token::Comma => { self.advance(); }
                Token::RParen => break,
                _ => return Err(ParseError::Syntax { offset: self.peek_offset(), message: "expected ',' or ')'".into() }),
            }
        }
        self.expect(&Token::RParen)?;
        Ok(Statement::CreateTable { name, columns })
    }

    fn parse_drop(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::Drop)?;
        let offset = self.peek_offset();
        match self.advance() {
            Token::Table => Ok(Statement::DropTable { name: self.expect_identifier()? }),
            Token::Index => Err(ParseError::Syntax { offset, message: "DROP INDEX not yet implemented".into() }),
            other => Err(ParseError::Syntax { offset, message: format!("expected TABLE or INDEX, found {other:?}") }),
        }
    }
}

pub fn parse(sql: &str) -> Result<Statement, ParseError> {
    let tokens = tokenize(sql)?;
    let mut parser = Parser::new(tokens);
    parser.parse_statement()
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test sql::parser::tests`
Expected: PASS (3 tests)

- [ ] **Step 6: Commit**

```bash
git add src/sql/ast.rs src/sql/parser.rs
git commit -m "Add AST types and parser for CREATE TABLE / DROP TABLE"
```

---

## Task 21: Parser — INSERT

**Files:**
- Modify: `src/sql/parser.rs`

**Interfaces:**
- Consumes: everything from Task 20.
- Produces: `parse_primary_expr` (literal/column/paren expressions, reused by Task 22's WHERE parsing), `INSERT` statement parsing wired into `parse_statement`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src/sql/parser.rs`:

```rust
    #[test]
    fn parses_insert_with_explicit_columns() {
        let stmt = parse("INSERT INTO users (id, name) VALUES (1, 'Ada')").unwrap();
        assert_eq!(
            stmt,
            Statement::Insert {
                table: "users".into(),
                columns: Some(vec!["id".into(), "name".into()]),
                rows: vec![vec![Expr::IntLiteral(1), Expr::StringLiteral("Ada".into())]],
            }
        );
    }

    #[test]
    fn parses_insert_multiple_rows_without_columns() {
        let stmt = parse("INSERT INTO t VALUES (1, NULL), (2, TRUE)").unwrap();
        assert_eq!(
            stmt,
            Statement::Insert {
                table: "t".into(),
                columns: None,
                rows: vec![
                    vec![Expr::IntLiteral(1), Expr::Null],
                    vec![Expr::IntLiteral(2), Expr::BoolLiteral(true)],
                ],
            }
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test sql::parser::tests::parses_insert_with_explicit_columns`
Expected: FAIL — `Token::Insert` not handled in `parse_statement`.

- [ ] **Step 3: Implement `parse_insert` and `parse_primary_expr`**

Add `Token::Insert => self.parse_insert()?,` as a new match arm in `parse_statement`'s `match self.peek()`.

Add to `impl Parser`:

```rust
    fn parse_primary_expr(&mut self) -> Result<Expr, ParseError> {
        let offset = self.peek_offset();
        match self.advance() {
            Token::IntLiteral(n) => Ok(Expr::IntLiteral(n)),
            Token::StringLiteral(s) => Ok(Expr::StringLiteral(s)),
            Token::True => Ok(Expr::BoolLiteral(true)),
            Token::False => Ok(Expr::BoolLiteral(false)),
            Token::Null => Ok(Expr::Null),
            Token::Identifier(name) => Ok(Expr::Column(name)),
            Token::LParen => {
                let e = self.parse_where_expr()?;
                self.expect(&Token::RParen)?;
                Ok(e)
            }
            other => Err(ParseError::Syntax { offset, message: format!("expected an expression, found {other:?}") }),
        }
    }

    fn parse_insert(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::Insert)?;
        self.expect(&Token::Into)?;
        let table = self.expect_identifier()?;

        let columns = if matches!(self.peek(), Token::LParen) {
            self.advance();
            let mut cols = Vec::new();
            loop {
                cols.push(self.expect_identifier()?);
                match self.peek() {
                    Token::Comma => { self.advance(); }
                    Token::RParen => break,
                    _ => return Err(ParseError::Syntax { offset: self.peek_offset(), message: "expected ',' or ')'".into() }),
                }
            }
            self.expect(&Token::RParen)?;
            Some(cols)
        } else {
            None
        };

        self.expect(&Token::Values)?;
        let mut rows = Vec::new();
        loop {
            self.expect(&Token::LParen)?;
            let mut row = Vec::new();
            loop {
                row.push(self.parse_primary_expr()?);
                match self.peek() {
                    Token::Comma => { self.advance(); }
                    Token::RParen => break,
                    _ => return Err(ParseError::Syntax { offset: self.peek_offset(), message: "expected ',' or ')'".into() }),
                }
            }
            self.expect(&Token::RParen)?;
            rows.push(row);
            match self.peek() {
                Token::Comma => { self.advance(); }
                _ => break,
            }
        }
        Ok(Statement::Insert { table, columns, rows })
    }

    fn parse_where_expr(&mut self) -> Result<Expr, ParseError> {
        // Full precedence-climbing implementation lands in Task 22; for now this
        // only needs to support parenthesized literal/column expressions used by
        // parse_primary_expr's `LParen` arm, so delegate straight to primary.
        self.parse_primary_expr()
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test sql::parser::tests`
Expected: PASS (5 tests)

- [ ] **Step 5: Commit**

```bash
git add src/sql/parser.rs
git commit -m "Add parser support for INSERT with literal expressions"
```

---

## Task 22: Parser — WHERE (precedence climbing), SELECT

**Files:**
- Modify: `src/sql/parser.rs`

**Interfaces:**
- Consumes: everything from Task 21.
- Produces: real `parse_where_expr` (replacing Task 21's placeholder), `SELECT` statement parsing with projection, `WHERE`, `ORDER BY`, `LIMIT`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module:

```rust
    #[test]
    fn parses_select_star_with_where_and_precedence() {
        // Confirms AND binds tighter than OR: `a OR b AND c` parses as `a OR (b AND c)`.
        let stmt = parse("SELECT * FROM t WHERE a = 1 OR b = 2 AND c = 3").unwrap();
        match stmt {
            Statement::Select { where_clause: Some(Expr::BinaryOp { op: BinOp::Or, left, right }), .. } => {
                assert_eq!(*left, Expr::BinaryOp {
                    op: BinOp::Eq,
                    left: Box::new(Expr::Column("a".into())),
                    right: Box::new(Expr::IntLiteral(1)),
                });
                assert!(matches!(*right, Expr::BinaryOp { op: BinOp::And, .. }));
            }
            other => panic!("unexpected parse result: {other:?}"),
        }
    }

    #[test]
    fn parses_select_with_is_null() {
        let stmt = parse("SELECT id FROM t WHERE name IS NOT NULL").unwrap();
        assert_eq!(
            stmt,
            Statement::Select {
                columns: SelectColumns::List(vec!["id".into()]),
                table: "t".into(),
                where_clause: Some(Expr::IsNull { expr: Box::new(Expr::Column("name".into())), negated: true }),
                order_by: None,
                limit: None,
            }
        );
    }

    #[test]
    fn parses_select_with_order_by_and_limit() {
        let stmt = parse("SELECT * FROM t ORDER BY id DESC LIMIT 10").unwrap();
        match stmt {
            Statement::Select { order_by, limit, .. } => {
                assert_eq!(order_by, Some(("id".into(), true)));
                assert_eq!(limit, Some(10));
            }
            other => panic!("unexpected parse result: {other:?}"),
        }
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test sql::parser::tests::parses_select_star_with_where_and_precedence`
Expected: FAIL — `Token::Select` not handled in `parse_statement`, and `parse_where_expr` is still the Task 21 placeholder.

- [ ] **Step 3: Implement real `parse_where_expr` and `parse_select`**

Add `Token::Select => self.parse_select()?,` as a new match arm in `parse_statement`.

Replace the placeholder `parse_where_expr` from Task 21 with:

```rust
    fn parse_where_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_and()?;
        while matches!(self.peek(), Token::Or) {
            self.advance();
            let right = self.parse_and()?;
            left = Expr::BinaryOp { op: BinOp::Or, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_comparison()?;
        while matches!(self.peek(), Token::And) {
            self.advance();
            let right = self.parse_comparison()?;
            left = Expr::BinaryOp { op: BinOp::And, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expr, ParseError> {
        let left = self.parse_primary_expr()?;
        match self.peek() {
            Token::Eq | Token::NotEq | Token::Lt | Token::LtEq | Token::Gt | Token::GtEq => {
                let op = match self.advance() {
                    Token::Eq => BinOp::Eq,
                    Token::NotEq => BinOp::NotEq,
                    Token::Lt => BinOp::Lt,
                    Token::LtEq => BinOp::LtEq,
                    Token::Gt => BinOp::Gt,
                    Token::GtEq => BinOp::GtEq,
                    _ => unreachable!(),
                };
                let right = self.parse_primary_expr()?;
                Ok(Expr::BinaryOp { op, left: Box::new(left), right: Box::new(right) })
            }
            Token::Is => {
                self.advance();
                let negated = if matches!(self.peek(), Token::Not) {
                    self.advance();
                    true
                } else {
                    false
                };
                self.expect(&Token::Null)?;
                Ok(Expr::IsNull { expr: Box::new(left), negated })
            }
            _ => Ok(left),
        }
    }

    fn parse_select(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::Select)?;
        let columns = if matches!(self.peek(), Token::Star) {
            self.advance();
            SelectColumns::All
        } else {
            let mut cols = vec![self.expect_identifier()?];
            while matches!(self.peek(), Token::Comma) {
                self.advance();
                cols.push(self.expect_identifier()?);
            }
            SelectColumns::List(cols)
        };
        self.expect(&Token::From)?;
        let table = self.expect_identifier()?;

        let where_clause = if matches!(self.peek(), Token::Where) {
            self.advance();
            Some(self.parse_where_expr()?)
        } else {
            None
        };

        let order_by = if matches!(self.peek(), Token::Order) {
            self.advance();
            self.expect(&Token::By)?;
            let col = self.expect_identifier()?;
            let desc = match self.peek() {
                Token::Desc => { self.advance(); true }
                Token::Asc => { self.advance(); false }
                _ => false,
            };
            Some((col, desc))
        } else {
            None
        };

        let limit = if matches!(self.peek(), Token::Limit) {
            self.advance();
            let offset = self.peek_offset();
            match self.advance() {
                Token::IntLiteral(n) => Some(n),
                other => return Err(ParseError::Syntax { offset, message: format!("expected integer after LIMIT, found {other:?}") }),
            }
        } else {
            None
        };

        Ok(Statement::Select { columns, table, where_clause, order_by, limit })
    }
```

Note: `parse_and` now calls `parse_comparison` directly (there is no standalone `NOT expr` production in this grammar — `NOT` only appears inside `IS NOT NULL`, which `parse_comparison` already handles). This is simpler than the originally sketched `parse_or → parse_and → parse_not → parse_comparison` chain and is the correct final design — do not add an empty `parse_not` passthrough layer.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test sql::parser::tests`
Expected: PASS (8 tests)

- [ ] **Step 5: Commit**

```bash
git add src/sql/parser.rs
git commit -m "Add precedence-climbing WHERE parser and SELECT statement parsing"
```

---

## Task 23: Parser — UPDATE, DELETE, CREATE INDEX, DROP INDEX

**Files:**
- Modify: `src/sql/parser.rs`

**Interfaces:**
- Consumes: everything from Task 22.
- Produces: full grammar coverage — every `Statement` variant is now parseable.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module:

```rust
    #[test]
    fn parses_update() {
        let stmt = parse("UPDATE t SET name = 'Bea', active = FALSE WHERE id = 1").unwrap();
        assert_eq!(
            stmt,
            Statement::Update {
                table: "t".into(),
                assignments: vec![
                    ("name".into(), Expr::StringLiteral("Bea".into())),
                    ("active".into(), Expr::BoolLiteral(false)),
                ],
                where_clause: Some(Expr::BinaryOp {
                    op: BinOp::Eq,
                    left: Box::new(Expr::Column("id".into())),
                    right: Box::new(Expr::IntLiteral(1)),
                }),
            }
        );
    }

    #[test]
    fn parses_delete() {
        let stmt = parse("DELETE FROM t WHERE id = 1").unwrap();
        assert_eq!(
            stmt,
            Statement::Delete {
                table: "t".into(),
                where_clause: Some(Expr::BinaryOp {
                    op: BinOp::Eq,
                    left: Box::new(Expr::Column("id".into())),
                    right: Box::new(Expr::IntLiteral(1)),
                }),
            }
        );
    }

    #[test]
    fn parses_create_index_and_drop_index() {
        assert_eq!(
            parse("CREATE INDEX idx_name ON t (name)").unwrap(),
            Statement::CreateIndex { name: "idx_name".into(), table: "t".into(), column: "name".into() }
        );
        assert_eq!(parse("DROP INDEX idx_name").unwrap(), Statement::DropIndex { name: "idx_name".into() });
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test sql::parser::tests::parses_update`
Expected: FAIL — `Token::Update`/`Token::Delete` not handled in `parse_statement`; `parse_create`/`parse_drop` reject `INDEX`.

- [ ] **Step 3: Implement**

Add `Token::Update => self.parse_update()?,` and `Token::Delete => self.parse_delete()?,` as new match arms in `parse_statement`.

Replace the `Token::Index => Err(...)` arm in `parse_create` with `Token::Index => self.parse_create_index(),`.

Replace the `Token::Index => Err(...)` arm in `parse_drop` with `Token::Index => Ok(Statement::DropIndex { name: self.expect_identifier()? }),`.

Add to `impl Parser`:

```rust
    fn parse_create_index(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::Index)?;
        let name = self.expect_identifier()?;
        self.expect(&Token::On)?;
        let table = self.expect_identifier()?;
        self.expect(&Token::LParen)?;
        let column = self.expect_identifier()?;
        self.expect(&Token::RParen)?;
        Ok(Statement::CreateIndex { name, table, column })
    }

    fn parse_update(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::Update)?;
        let table = self.expect_identifier()?;
        self.expect(&Token::Set)?;
        let mut assignments = Vec::new();
        loop {
            let col = self.expect_identifier()?;
            self.expect(&Token::Eq)?;
            let value = self.parse_primary_expr()?;
            assignments.push((col, value));
            match self.peek() {
                Token::Comma => { self.advance(); }
                _ => break,
            }
        }
        let where_clause = if matches!(self.peek(), Token::Where) {
            self.advance();
            Some(self.parse_where_expr()?)
        } else {
            None
        };
        Ok(Statement::Update { table, assignments, where_clause })
    }

    fn parse_delete(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::Delete)?;
        self.expect(&Token::From)?;
        let table = self.expect_identifier()?;
        let where_clause = if matches!(self.peek(), Token::Where) {
            self.advance();
            Some(self.parse_where_expr()?)
        } else {
            None
        };
        Ok(Statement::Delete { table, where_clause })
    }
```

- [ ] **Step 4: Run the full parser test suite**

Run: `cargo test sql::`
Expected: PASS (all tests from Tasks 19–23; 11 parser tests plus 7 lexer tests, 18 total — the lexer count grew from 5 to 7 in Task 19 when its reviewer required non-ASCII UTF-8 coverage)

- [ ] **Step 5: Commit**

```bash
git add src/sql/parser.rs
git commit -m "Complete parser grammar: UPDATE, DELETE, CREATE INDEX, DROP INDEX"
```

---

## Task 24: Expression evaluation

**Files:**
- Create: `src/plan/expr.rs`

**Interfaces:**
- Consumes: `ast::{Expr, BinOp}`, `schema::TableSchema`, `value::{Value, sql_cmp, sql_cmp_nullable}`, `PlanError`.
- Produces: `expr::{eval, is_truthy}` — `eval(&Expr, &TableSchema, &[Value]) -> Result<Value, PlanError>`, `is_truthy(&Value) -> bool`.

- [ ] **Step 1: Write the failing test**

Write `src/plan/expr.rs`:

```rust
use crate::error::PlanError;
use crate::sql::ast::{BinOp, Expr};
use crate::types::schema::TableSchema;
use crate::types::value::{sql_cmp, Value};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::schema::Column;
    use crate::types::value::ColumnType;

    fn schema() -> TableSchema {
        TableSchema {
            name: "t".into(),
            columns: vec![
                Column { name: "id".into(), ty: ColumnType::Integer, not_null: true, is_primary_key: true },
                Column { name: "name".into(), ty: ColumnType::Text, not_null: false, is_primary_key: false },
            ],
            root_page: 0,
        }
    }

    #[test]
    fn evaluates_column_reference() {
        let row = vec![Value::Integer(7), Value::Text("x".into())];
        assert_eq!(eval(&Expr::Column("id".into()), &schema(), &row).unwrap(), Value::Integer(7));
    }

    #[test]
    fn unknown_column_errors() {
        let row = vec![Value::Integer(7), Value::Text("x".into())];
        assert!(matches!(eval(&Expr::Column("nope".into()), &schema(), &row), Err(PlanError::NoSuchColumn(_))));
    }

    #[test]
    fn equality_comparison() {
        let row = vec![Value::Integer(7), Value::Text("x".into())];
        let expr = Expr::BinaryOp {
            op: BinOp::Eq,
            left: Box::new(Expr::Column("id".into())),
            right: Box::new(Expr::IntLiteral(7)),
        };
        assert_eq!(eval(&expr, &schema(), &row).unwrap(), Value::Boolean(true));
    }

    #[test]
    fn comparison_against_null_is_false_not_panic() {
        let row = vec![Value::Integer(7), Value::Null];
        let expr = Expr::BinaryOp {
            op: BinOp::Eq,
            left: Box::new(Expr::Column("name".into())),
            right: Box::new(Expr::StringLiteral("x".into())),
        };
        assert_eq!(eval(&expr, &schema(), &row).unwrap(), Value::Boolean(false));
    }

    #[test]
    fn is_null_and_is_not_null() {
        let row = vec![Value::Integer(7), Value::Null];
        let is_null = Expr::IsNull { expr: Box::new(Expr::Column("name".into())), negated: false };
        let is_not_null = Expr::IsNull { expr: Box::new(Expr::Column("name".into())), negated: true };
        assert_eq!(eval(&is_null, &schema(), &row).unwrap(), Value::Boolean(true));
        assert_eq!(eval(&is_not_null, &schema(), &row).unwrap(), Value::Boolean(false));
    }

    #[test]
    fn and_or_short_circuit_semantics() {
        let row = vec![Value::Integer(7), Value::Text("x".into())];
        let and_expr = Expr::BinaryOp {
            op: BinOp::And,
            left: Box::new(Expr::BoolLiteral(true)),
            right: Box::new(Expr::BoolLiteral(false)),
        };
        assert_eq!(eval(&and_expr, &schema(), &row).unwrap(), Value::Boolean(false));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test plan::expr::tests`
Expected: FAIL — `eval` not defined.

- [ ] **Step 3: Implement**

Add above the test module:

```rust
pub fn eval(expr: &Expr, schema: &TableSchema, row: &[Value]) -> Result<Value, PlanError> {
    match expr {
        Expr::Column(name) => {
            let idx = schema.column_index(name).ok_or_else(|| PlanError::NoSuchColumn(name.clone()))?;
            Ok(row[idx].clone())
        }
        Expr::IntLiteral(i) => Ok(Value::Integer(*i)),
        Expr::StringLiteral(s) => Ok(Value::Text(s.clone())),
        Expr::BoolLiteral(b) => Ok(Value::Boolean(*b)),
        Expr::Null => Ok(Value::Null),
        Expr::IsNull { expr, negated } => {
            let v = eval(expr, schema, row)?;
            let is_null = matches!(v, Value::Null);
            Ok(Value::Boolean(is_null != *negated))
        }
        Expr::BinaryOp { op, left, right } => {
            let l = eval(left, schema, row)?;
            let r = eval(right, schema, row)?;
            Ok(eval_binop(*op, &l, &r))
        }
    }
}

fn eval_binop(op: BinOp, l: &Value, r: &Value) -> Value {
    match op {
        BinOp::And => Value::Boolean(is_truthy(l) && is_truthy(r)),
        BinOp::Or => Value::Boolean(is_truthy(l) || is_truthy(r)),
        _ => {
            if matches!(l, Value::Null) || matches!(r, Value::Null) {
                return Value::Boolean(false);
            }
            let ord = sql_cmp(l, r);
            let result = match op {
                BinOp::Eq => ord == std::cmp::Ordering::Equal,
                BinOp::NotEq => ord != std::cmp::Ordering::Equal,
                BinOp::Lt => ord == std::cmp::Ordering::Less,
                BinOp::LtEq => ord != std::cmp::Ordering::Greater,
                BinOp::Gt => ord == std::cmp::Ordering::Greater,
                BinOp::GtEq => ord != std::cmp::Ordering::Less,
                BinOp::And | BinOp::Or => unreachable!(),
            };
            Value::Boolean(result)
        }
    }
}

pub fn is_truthy(v: &Value) -> bool {
    matches!(v, Value::Boolean(true))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test plan::expr::tests`
Expected: PASS (6 tests)

- [ ] **Step 5: Commit**

```bash
git add src/plan/expr.rs
git commit -m "Add WHERE expression evaluation with NULL-as-false comparison semantics"
```

---

## Task 25: Executor — Operator trait, SeqScan, Filter, Project

**Files:**
- Modify: `src/exec.rs` (add the `Operator` trait — deferred here from Task 1, since it needs `Pager` from Task 3 and `Value` from Task 5)
- Create: `src/exec/scan.rs`
- Create: `src/exec/filter.rs`
- Create: `src/exec/project.rs`

**Interfaces:**
- Consumes: `Pager`, `btree::tree::BTree`, `btree::cursor::Cursor`, `row::decode_row`, `schema::TableSchema`, `plan::expr::{eval, is_truthy}`, `ast::Expr`.
- Produces: `exec::Operator` (trait, defined in this task); `scan::SeqScan::new(TableSchema) -> SeqScan`; `filter::Filter { input: Box<dyn Operator>, schema: TableSchema, predicate: Expr }`; `project::Project { input: Box<dyn Operator>, indices: Vec<usize> }`. All three implement `Operator`.

- [ ] **Step 0: Define the `Operator` trait**

`src/exec.rs` currently holds only the six `pub mod` lines from Task 1. Add the trait below them:

```rust
use crate::error::ExecError;
use crate::storage::pager::Pager;
use crate::types::value::Value;

pub trait Operator {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Vec<Value>>, ExecError>;
}
```

- [ ] **Step 1: Write the failing test for `SeqScan`**

Write `src/exec/scan.rs`:

```rust
use crate::btree::cursor::Cursor;
use crate::btree::tree::BTree;
use crate::error::ExecError;
use crate::exec::Operator;
use crate::storage::pager::Pager;
use crate::types::row::decode_row;
use crate::types::schema::TableSchema;
use crate::types::value::Value;

pub struct SeqScan {
    cursor: Cursor,
    root: u32,
    schema: TableSchema,
    started: bool,
}

impl SeqScan {
    pub fn new(schema: TableSchema) -> Self {
        let root = schema.root_page;
        SeqScan { cursor: Cursor::empty(), root, schema, started: false }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::btree::node::LeafNode;
    use crate::types::schema::Column;
    use crate::types::value::ColumnType;
    use tempfile::NamedTempFile;

    fn schema_with_root(root: u32) -> TableSchema {
        TableSchema {
            name: "t".into(),
            columns: vec![Column { name: "id".into(), ty: ColumnType::Integer, not_null: true, is_primary_key: true }],
            root_page: root,
        }
    }

    #[test]
    fn scans_all_rows_in_key_order() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let initial_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(initial_root).unwrap());
        let final_root = {
            let mut bt = BTree::new(&mut pager, initial_root);
            let schema = schema_with_root(initial_root);
            for i in [3, 1, 2] {
                let row = vec![Value::Integer(i)];
                bt.insert(
                    &crate::types::value::encode_key(&Value::Integer(i)),
                    &crate::types::row::encode_row(&schema, &row),
                )
                .unwrap();
            }
            bt.root()
        };

        let mut scan = SeqScan::new(schema_with_root(final_root));
        let mut seen = Vec::new();
        while let Some(row) = scan.next(&mut pager).unwrap() {
            seen.push(row[0].clone());
        }
        assert_eq!(seen, vec![Value::Integer(1), Value::Integer(2), Value::Integer(3)]);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test exec::scan::tests`
Expected: FAIL — `Operator` not implemented for `SeqScan`.

- [ ] **Step 3: Implement `SeqScan`**

Add to `src/exec/scan.rs`:

```rust
impl Operator for SeqScan {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Vec<Value>>, ExecError> {
        if !self.started {
            self.cursor = { BTree::new(pager, self.root).cursor_start()? };
            self.started = true;
        }
        match self.cursor.next(pager)? {
            Some((_key, payload)) => Ok(Some(decode_row(&self.schema, &payload))),
            None => Ok(None),
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test exec::scan::tests`
Expected: PASS

- [ ] **Step 5: Write and implement `Filter`**

Write `src/exec/filter.rs`:

```rust
use crate::error::ExecError;
use crate::exec::Operator;
use crate::plan::expr::{eval, is_truthy};
use crate::sql::ast::Expr;
use crate::storage::pager::Pager;
use crate::types::schema::TableSchema;
use crate::types::value::Value;

pub struct Filter {
    pub input: Box<dyn Operator>,
    pub schema: TableSchema,
    pub predicate: Expr,
}

impl Operator for Filter {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Vec<Value>>, ExecError> {
        loop {
            match self.input.next(pager)? {
                Some(row) => {
                    let v = eval(&self.predicate, &self.schema, &row)
                        .map_err(|e| ExecError::InvalidValue(e.to_string()))?;
                    if is_truthy(&v) {
                        return Ok(Some(row));
                    }
                }
                None => return Ok(None),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::scan::SeqScan;
    use crate::sql::ast::BinOp;
    use crate::types::schema::Column;
    use crate::types::value::ColumnType;

    // Filter is exercised end-to-end (with a real SeqScan feeding it) in Task 29's
    // integration test; this unit test only checks the truthiness/loop logic using
    // a trivial in-memory Operator stub so it doesn't need a Pager at all.
    struct Fixed(Vec<Vec<Value>>);
    impl Operator for Fixed {
        fn next(&mut self, _pager: &mut Pager) -> Result<Option<Vec<Value>>, ExecError> {
            Ok(self.0.pop())
        }
    }

    #[test]
    fn filters_out_non_matching_rows() {
        use tempfile::NamedTempFile;
        let file = NamedTempFile::new().unwrap();
        let mut pager = crate::storage::pager::Pager::create(file.path()).unwrap();

        let schema = TableSchema {
            name: "t".into(),
            columns: vec![Column { name: "id".into(), ty: ColumnType::Integer, not_null: true, is_primary_key: true }],
            root_page: 0,
        };
        // `Fixed` yields via Vec::pop(), which returns the LAST element first: this
        // list is consumed in the order 1, 2, 3 (not the 3, 2, 1 it's written in).
        let input = Fixed(vec![
            vec![Value::Integer(3)],
            vec![Value::Integer(2)],
            vec![Value::Integer(1)],
        ]);
        let predicate = Expr::BinaryOp {
            op: BinOp::Gt,
            left: Box::new(Expr::Column("id".into())),
            right: Box::new(Expr::IntLiteral(1)),
        };
        let mut filter = Filter { input: Box::new(input), schema, predicate };
        let mut seen = Vec::new();
        while let Some(row) = filter.next(&mut pager).unwrap() {
            seen.push(row[0].clone());
        }
        // Stream order is [1, 2, 3]; id > 1 excludes 1 and keeps 2 and 3.
        assert_eq!(seen, vec![Value::Integer(2), Value::Integer(3)]);
    }
}
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo test exec::filter::tests`
Expected: PASS

- [ ] **Step 7: Write and implement `Project`**

Write `src/exec/project.rs`:

```rust
use crate::error::ExecError;
use crate::exec::Operator;
use crate::storage::pager::Pager;
use crate::types::value::Value;

pub struct Project {
    pub input: Box<dyn Operator>,
    pub indices: Vec<usize>,
}

impl Operator for Project {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Vec<Value>>, ExecError> {
        match self.input.next(pager)? {
            Some(row) => Ok(Some(self.indices.iter().map(|&i| row[i].clone()).collect())),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(Vec<Vec<Value>>);
    impl Operator for Fixed {
        fn next(&mut self, _pager: &mut Pager) -> Result<Option<Vec<Value>>, ExecError> {
            Ok(self.0.pop())
        }
    }

    #[test]
    fn projects_selected_columns_in_order() {
        use tempfile::NamedTempFile;
        let file = NamedTempFile::new().unwrap();
        let mut pager = crate::storage::pager::Pager::create(file.path()).unwrap();

        let input = Fixed(vec![vec![Value::Integer(1), Value::Text("a".into()), Value::Boolean(true)]]);
        let mut project = Project { input: Box::new(input), indices: vec![2, 0] };
        let row = project.next(&mut pager).unwrap().unwrap();
        assert_eq!(row, vec![Value::Boolean(true), Value::Integer(1)]);
    }
}
```

- [ ] **Step 8: Run all executor tests**

Run: `cargo test exec::`
Expected: PASS (5 tests across scan/filter/project)

- [ ] **Step 9: Commit**

```bash
git add src/exec/scan.rs src/exec/filter.rs src/exec/project.rs
git commit -m "Add SeqScan, Filter, and Project operators"
```

---

## Task 26: Engine struct — CREATE TABLE / DROP TABLE

**Files:**
- Modify: `src/engine.rs`

**Interfaces:**
- Consumes: `Pager`, `Catalog`, `sql::parser::parse`, `sql::ast::{Statement, ColumnDef}`, `schema::{Column, TableSchema}`, `btree::node::LeafNode`.
- Produces: `engine::{Database, ExecResult}`. `Database::create(&Path) -> Result<Database>`, `Database::open(&Path) -> Result<Database>`, `Database::execute(&mut self, sql: &str) -> Result<ExecResult>`. `ExecResult::{Rows { columns: Vec<String>, rows: Vec<Vec<Value>> }, Modified(usize), Ok}`. Every later engine task (27–36) adds one more `Statement` match arm and its `execute_*` method to this same file.

- [ ] **Step 1: Write the failing test**

Replace the placeholder `src/engine.rs` (currently just `pub struct Database;` from Task 1) with:

```rust
use std::path::Path;

use crate::btree::node::LeafNode;
use crate::catalog::Catalog;
use crate::error::{DbError, PlanError, Result};
use crate::sql::ast::{ColumnDef, Statement};
use crate::storage::pager::Pager;
use crate::types::schema::{Column, TableSchema};
use crate::types::value::Value;

pub struct Database {
    pager: Pager,
    catalog: Catalog,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExecResult {
    Rows { columns: Vec<String>, rows: Vec<Vec<Value>> },
    Modified(usize),
    Ok,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn create_table_then_drop_table() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        assert_eq!(
            db.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)").unwrap(),
            ExecResult::Ok
        );
        assert_eq!(db.execute("DROP TABLE users").unwrap(), ExecResult::Ok);
    }

    #[test]
    fn create_table_without_primary_key_errors() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        let err = db.execute("CREATE TABLE t (a INTEGER)").unwrap_err();
        assert!(matches!(err, DbError::Plan(PlanError::InvalidSchema(_))));
    }

    #[test]
    fn create_duplicate_table_errors() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)").unwrap();
        let err = db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)").unwrap_err();
        assert!(matches!(err, DbError::Plan(PlanError::TableAlreadyExists(_))));
    }

    #[test]
    fn reopening_preserves_schema() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path();
        {
            let mut db = Database::create(path).unwrap();
            db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)").unwrap();
        }
        let mut db = Database::open(path).unwrap();
        db.execute("DROP TABLE t").unwrap(); // succeeds only if the schema survived reopen
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test engine::tests`
Expected: FAIL — `Database::create`/`execute` not defined.

- [ ] **Step 3: Implement**

Add above the test module:

```rust
impl Database {
    pub fn create(path: &Path) -> Result<Self> {
        let mut pager = Pager::create(path)?;
        let catalog = Catalog::bootstrap(&mut pager)?;
        Ok(Database { pager, catalog })
    }

    pub fn open(path: &Path) -> Result<Self> {
        let mut pager = Pager::open(path)?;
        let catalog = Catalog::bootstrap(&mut pager)?;
        Ok(Database { pager, catalog })
    }

    pub fn execute(&mut self, sql: &str) -> Result<ExecResult> {
        let stmt = crate::sql::parser::parse(sql)?;
        let result = match stmt {
            Statement::CreateTable { name, columns } => self.execute_create_table(name, columns)?,
            Statement::DropTable { name } => self.execute_drop_table(&name)?,
            other => {
                return Err(DbError::Plan(PlanError::InvalidSchema(format!(
                    "statement not yet supported: {other:?}"
                ))))
            }
        };
        self.pager.flush()?;
        Ok(result)
    }

    fn execute_create_table(&mut self, name: String, columns: Vec<ColumnDef>) -> Result<ExecResult> {
        let pk_count = columns.iter().filter(|c| c.primary_key).count();
        if pk_count != 1 {
            return Err(DbError::Plan(PlanError::InvalidSchema(format!(
                "table {name} must declare exactly one PRIMARY KEY column"
            ))));
        }
        let root = self.pager.allocate_page()?;
        LeafNode { entries: vec![], next_leaf: 0 }.encode(self.pager.get_page_mut(root)?);
        let cols = columns
            .into_iter()
            .map(|c| Column {
                name: c.name,
                ty: c.ty,
                not_null: c.not_null || c.primary_key,
                is_primary_key: c.primary_key,
            })
            .collect();
        let schema = TableSchema { name, columns: cols, root_page: root };
        self.catalog.create_table(&mut self.pager, &schema)?;
        Ok(ExecResult::Ok)
    }

    fn execute_drop_table(&mut self, name: &str) -> Result<ExecResult> {
        self.catalog.drop_table(&mut self.pager, name)?;
        Ok(ExecResult::Ok)
    }
}
```

Also update `src/lib.rs`'s `pub use engine::Database;` to `pub use engine::{Database, ExecResult};` so later integration tests (Task 29 onward) can match on the result variant from outside the crate.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test engine::tests`
Expected: PASS (4 tests)

- [ ] **Step 5: Commit**

```bash
git add src/engine.rs src/lib.rs
git commit -m "Add Database engine with CREATE TABLE / DROP TABLE execution"
```

---

## Task 27: Engine — INSERT execution

**Files:**
- Create: `src/exec/mutate.rs`
- Modify: `src/engine.rs`

**Interfaces:**
- Consumes: everything from Task 26, `btree::tree::BTree`, `row::encode_row`, `IndexSchema`.
- Produces: `mutate::insert_row(&mut Pager, &TableSchema, &[IndexSchema], &[Value]) -> Result<(u32, Vec<(String, u32)>), ExecError>` returning `(new_table_root, new_index_roots)` — every mutating engine method from here on follows this "return new roots, caller persists them" pattern (see Global Constraints). `Database::execute` gains the `Insert` match arm.

- [ ] **Step 1: Write the failing test for `insert_row`**

Write `src/exec/mutate.rs`:

```rust
use crate::btree::tree::BTree;
use crate::error::{BTreeError, ExecError};
use crate::storage::pager::Pager;
use crate::types::row::encode_row;
use crate::types::schema::{IndexSchema, TableSchema};
use crate::types::value::{encode_composite_key, encode_key, Value};

pub fn insert_row(
    pager: &mut Pager,
    schema: &TableSchema,
    indexes: &[IndexSchema],
    row: &[Value],
) -> Result<(u32, Vec<(String, u32)>), ExecError> {
    todo_marker(); // placeholder removed in Step 3
    unreachable!()
}

fn todo_marker() {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::btree::node::LeafNode;
    use crate::types::schema::Column;
    use crate::types::value::ColumnType;
    use tempfile::NamedTempFile;

    fn schema(root: u32) -> TableSchema {
        TableSchema {
            name: "t".into(),
            columns: vec![
                Column { name: "id".into(), ty: ColumnType::Integer, not_null: true, is_primary_key: true },
                Column { name: "name".into(), ty: ColumnType::Text, not_null: true, is_primary_key: false },
            ],
            root_page: root,
        }
    }

    #[test]
    fn inserts_row_and_reports_unchanged_root_for_small_table() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(root).unwrap());
        let s = schema(root);

        let (new_root, new_index_roots) = insert_row(&mut pager, &s, &[], &[Value::Integer(1), Value::Text("a".into())]).unwrap();
        assert_eq!(new_root, root);
        assert!(new_index_roots.is_empty());

        let mut bt = BTree::new(&mut pager, new_root);
        let payload = bt.search(&crate::types::value::encode_key(&Value::Integer(1))).unwrap().unwrap();
        assert_eq!(crate::types::row::decode_row(&s, &payload), vec![Value::Integer(1), Value::Text("a".into())]);
    }

    #[test]
    fn not_null_violation_is_rejected() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(root).unwrap());
        let s = schema(root);
        let err = insert_row(&mut pager, &s, &[], &[Value::Integer(1), Value::Null]).unwrap_err();
        assert!(matches!(err, ExecError::NotNullViolation(_)));
    }

    #[test]
    fn duplicate_primary_key_is_rejected() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(root).unwrap());
        let s = schema(root);
        insert_row(&mut pager, &s, &[], &[Value::Integer(1), Value::Text("a".into())]).unwrap();
        let err = insert_row(&mut pager, &s, &[], &[Value::Integer(1), Value::Text("b".into())]).unwrap_err();
        assert!(matches!(err, ExecError::DuplicatePrimaryKey));
    }

    #[test]
    fn interior_nul_in_text_is_rejected() {
        // TEXT keys are encoded as UTF-8 bytes + a 0x00 terminator (types/value.rs).
        // A value containing an embedded NUL byte would let the terminator-based
        // encoding stop early, breaking order-preservation for composite keys built
        // from it — see the design spec's storage section: "Text values may not
        // contain an interior NUL; this is validated at insert and rejected as
        // InvalidValue." This is that validation.
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(root).unwrap());
        let s = schema(root);
        let err = insert_row(&mut pager, &s, &[], &[Value::Integer(1), Value::Text("a\0b".into())]).unwrap_err();
        assert!(matches!(err, ExecError::InvalidValue(_)));
    }

    #[test]
    fn maintains_secondary_index_entries() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let table_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(table_root).unwrap());
        let index_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(index_root).unwrap());
        let s = schema(table_root);
        let idx = IndexSchema { name: "idx_name".into(), table: "t".into(), column: "name".into(), root_page: index_root };

        let (_, new_index_roots) = insert_row(&mut pager, &s, &[idx.clone()], &[Value::Integer(1), Value::Text("a".into())]).unwrap();
        let final_index_root = new_index_roots[0].1;

        let mut ibt = BTree::new(&mut pager, final_index_root);
        let idx_key = crate::types::value::encode_composite_key(&[Value::Text("a".into()), Value::Integer(1)]);
        assert_eq!(ibt.search(&idx_key).unwrap(), Some(vec![]));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test exec::mutate::tests`
Expected: FAIL — `insert_row` is `unreachable!()`.

- [ ] **Step 3: Implement `insert_row`**

Replace the placeholder body:

```rust
pub fn insert_row(
    pager: &mut Pager,
    schema: &TableSchema,
    indexes: &[IndexSchema],
    row: &[Value],
) -> Result<(u32, Vec<(String, u32)>), ExecError> {
    for (col, v) in schema.columns.iter().zip(row.iter()) {
        if col.not_null && matches!(v, Value::Null) {
            return Err(ExecError::NotNullViolation(col.name.clone()));
        }
        if let Value::Text(s) = v {
            if s.contains('\0') {
                return Err(ExecError::InvalidValue(format!(
                    "{} contains an interior NUL byte, which cannot be represented in an order-preserving key",
                    col.name
                )));
            }
        }
    }
    let pk_idx = schema.primary_key_index();
    let key = encode_key(&row[pk_idx]);
    let payload = encode_row(schema, row);
    let mut bt = BTree::new(pager, schema.root_page);
    bt.insert(&key, &payload).map_err(|e| match e {
        BTreeError::DuplicateKey => ExecError::DuplicatePrimaryKey,
        other => ExecError::BTree(other),
    })?;
    let new_table_root = bt.root();

    let mut new_index_roots = Vec::new();
    for idx in indexes {
        let col_idx = schema.column_index(&idx.column).expect("index column must exist in table schema");
        let idx_key = encode_composite_key(&[row[col_idx].clone(), row[pk_idx].clone()]);
        let mut ibt = BTree::new(pager, idx.root_page);
        ibt.insert(&idx_key, &[])?;
        new_index_roots.push((idx.name.clone(), ibt.root()));
    }
    Ok((new_table_root, new_index_roots))
}
```

Delete the now-unused `fn todo_marker() {}` helper.

The interior-NUL check applies to every `TEXT` column, not only the primary key or currently-indexed columns. `CREATE INDEX` (Task 34) builds an index from a full scan of whatever rows already exist, so a NUL-containing value in a column that is unindexed *today* would silently corrupt ordering the moment an index is added on that column later. Rejecting it unconditionally at insert time — matching the design spec's stated rule exactly — closes that gap regardless of which columns end up indexed.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test exec::mutate::tests`
Expected: PASS (5 tests)

- [ ] **Step 5: Wire `Insert` into the engine**

Add to `src/engine.rs`'s imports: `use crate::sql::ast::Expr;` and `use crate::types::schema::IndexSchema;` and `use std::collections::HashMap;`.

Add `Statement::Insert { table, columns, rows } => self.execute_insert(&table, columns, rows)?,` as a new match arm in `execute`, before the catch-all `other => ...` arm.

Add to `impl Database`:

```rust
    fn execute_insert(
        &mut self,
        table: &str,
        columns: Option<Vec<String>>,
        rows: Vec<Vec<Expr>>,
    ) -> Result<ExecResult> {
        let schema = self
            .catalog
            .get_table(&mut self.pager, table)?
            .ok_or_else(|| PlanError::NoSuchTable(table.to_string()))?;
        let indexes = self.catalog.list_indexes_for_table(&mut self.pager, table)?;

        let target_indices: Vec<usize> = match &columns {
            Some(names) => {
                let mut idxs = Vec::new();
                for n in names {
                    idxs.push(schema.column_index(n).ok_or_else(|| PlanError::NoSuchColumn(n.clone()))?);
                }
                idxs
            }
            None => (0..schema.columns.len()).collect(),
        };

        let mut count = 0usize;
        let mut table_root = schema.root_page;
        let mut index_roots: HashMap<String, u32> =
            indexes.iter().map(|i| (i.name.clone(), i.root_page)).collect();

        for row_exprs in rows {
            if row_exprs.len() != target_indices.len() {
                return Err(DbError::Exec(crate::error::ExecError::InvalidValue(
                    "value count does not match column count".into(),
                )));
            }
            let mut full_row = vec![Value::Null; schema.columns.len()];
            for (expr, &col_idx) in row_exprs.iter().zip(target_indices.iter()) {
                full_row[col_idx] = literal_to_value(expr)?;
            }

            let mut schema_for_write = schema.clone();
            schema_for_write.root_page = table_root;
            let indexes_for_write: Vec<IndexSchema> = indexes
                .iter()
                .cloned()
                .map(|mut idx| {
                    idx.root_page = index_roots[&idx.name];
                    idx
                })
                .collect();

            let (new_table_root, new_index_roots) =
                crate::exec::mutate::insert_row(&mut self.pager, &schema_for_write, &indexes_for_write, &full_row)?;
            table_root = new_table_root;
            for (name, root) in new_index_roots {
                index_roots.insert(name, root);
            }
            count += 1;
        }

        if table_root != schema.root_page {
            self.catalog.update_table_root(&mut self.pager, table, table_root)?;
        }
        for idx in &indexes {
            let new_root = index_roots[&idx.name];
            if new_root != idx.root_page {
                self.catalog.update_index_root(&mut self.pager, &idx.name, new_root)?;
            }
        }

        Ok(ExecResult::Modified(count))
    }
```

Add a free function at the bottom of `src/engine.rs`:

```rust
fn literal_to_value(expr: &Expr) -> Result<Value> {
    match expr {
        Expr::IntLiteral(n) => Ok(Value::Integer(*n)),
        Expr::StringLiteral(s) => Ok(Value::Text(s.clone())),
        Expr::BoolLiteral(b) => Ok(Value::Boolean(*b)),
        Expr::Null => Ok(Value::Null),
        other => Err(DbError::Exec(crate::error::ExecError::InvalidValue(format!(
            "expected a literal value in INSERT, found {other:?}"
        )))),
    }
}
```

- [ ] **Step 6: Write and run an engine-level INSERT test**

Add to `src/engine.rs`'s `tests` module:

```rust
    #[test]
    fn insert_then_reinsert_same_pk_errors() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)").unwrap();
        assert_eq!(
            db.execute("INSERT INTO t (id, name) VALUES (1, 'a')").unwrap(),
            ExecResult::Modified(1)
        );
        assert!(db.execute("INSERT INTO t (id, name) VALUES (1, 'b')").is_err());
    }

    #[test]
    fn insert_many_rows_forces_table_split_and_still_works() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)").unwrap();
        for i in 0..500 {
            let sql = format!("INSERT INTO t (id) VALUES ({i})");
            assert_eq!(db.execute(&sql).unwrap(), ExecResult::Modified(1));
        }
    }
```

Run: `cargo test engine::tests`
Expected: PASS (6 tests)

- [ ] **Step 7: Commit**

```bash
git add src/exec/mutate.rs src/engine.rs
git commit -m "Add INSERT execution with NOT NULL, PK uniqueness, and index maintenance"
```

---

## Task 28: Planner and engine — SELECT (SeqScan + Filter + Project)

**Files:**
- Modify: `src/plan/planner.rs` (declared empty via `src/plan.rs`'s `pub mod planner;` from Task 1)
- Modify: `src/engine.rs`

**Interfaces:**
- Consumes: `exec::{Operator, scan::SeqScan, filter::Filter, project::Project}`, `ast::Expr`, `schema::TableSchema`.
- Produces: `planner::build_select_plan(&TableSchema, Option<Expr>, Vec<usize>) -> Box<dyn Operator>`. `Database::execute` gains the `Select` match arm. `ORDER BY`/`LIMIT` are parsed but intentionally not yet applied — Task 31 completes that.

- [ ] **Step 1: Write the failing test for the planner**

Write `src/plan/planner.rs`:

```rust
use crate::exec::filter::Filter;
use crate::exec::project::Project;
use crate::exec::scan::SeqScan;
use crate::exec::Operator;
use crate::sql::ast::Expr;
use crate::types::schema::TableSchema;

pub fn build_select_plan(schema: &TableSchema, where_clause: Option<Expr>, projection_indices: Vec<usize>) -> Box<dyn Operator> {
    todo_marker();
    unreachable!()
}

fn todo_marker() {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::btree::node::LeafNode;
    use crate::storage::pager::Pager;
    use crate::types::schema::Column;
    use crate::types::value::{ColumnType, Value};
    use tempfile::NamedTempFile;

    #[test]
    fn builds_a_plan_that_scans_filters_and_projects() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let initial_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(initial_root).unwrap());

        let mut schema = TableSchema {
            name: "t".into(),
            columns: vec![
                Column { name: "id".into(), ty: ColumnType::Integer, not_null: true, is_primary_key: true },
                Column { name: "name".into(), ty: ColumnType::Text, not_null: true, is_primary_key: false },
            ],
            root_page: initial_root,
        };

        let final_root = {
            let mut bt = crate::btree::tree::BTree::new(&mut pager, initial_root);
            for (id, name) in [(1, "a"), (2, "b"), (3, "c")] {
                let row = vec![Value::Integer(id), Value::Text(name.into())];
                bt.insert(&crate::types::value::encode_key(&Value::Integer(id)), &crate::types::row::encode_row(&schema, &row)).unwrap();
            }
            bt.root()
        };
        schema.root_page = final_root;

        let predicate = crate::sql::ast::Expr::BinaryOp {
            op: crate::sql::ast::BinOp::Gt,
            left: Box::new(Expr::Column("id".into())),
            right: Box::new(Expr::IntLiteral(1)),
        };
        let mut plan = build_select_plan(&schema, Some(predicate), vec![1]); // project just "name"

        let mut rows = Vec::new();
        while let Some(row) = plan.next(&mut pager).unwrap() {
            rows.push(row);
        }
        assert_eq!(rows, vec![vec![Value::Text("b".into())], vec![Value::Text("c".into())]]);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test plan::planner::tests`
Expected: FAIL — `build_select_plan` is `unreachable!()`.

- [ ] **Step 3: Implement**

Replace the placeholder body:

```rust
pub fn build_select_plan(schema: &TableSchema, where_clause: Option<Expr>, projection_indices: Vec<usize>) -> Box<dyn Operator> {
    let mut plan: Box<dyn Operator> = Box::new(SeqScan::new(schema.clone()));
    if let Some(predicate) = where_clause {
        plan = Box::new(Filter { input: plan, schema: schema.clone(), predicate });
    }
    Box::new(Project { input: plan, indices: projection_indices })
}
```

Delete the unused `fn todo_marker() {}` helper.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test plan::planner::tests`
Expected: PASS

- [ ] **Step 5: Wire `Select` into the engine**

Add to `src/engine.rs`'s imports: `use crate::sql::ast::SelectColumns;`.

Add `Statement::Select { columns, table, where_clause, order_by: _order_by, limit: _limit } => self.execute_select(columns, &table, where_clause)?,` as a new match arm in `execute`.

Add to `impl Database`:

```rust
    fn execute_select(&mut self, columns: SelectColumns, table: &str, where_clause: Option<Expr>) -> Result<ExecResult> {
        let schema = self
            .catalog
            .get_table(&mut self.pager, table)?
            .ok_or_else(|| PlanError::NoSuchTable(table.to_string()))?;

        let (out_names, indices): (Vec<String>, Vec<usize>) = match &columns {
            SelectColumns::All => (
                schema.columns.iter().map(|c| c.name.clone()).collect(),
                (0..schema.columns.len()).collect(),
            ),
            SelectColumns::List(names) => {
                let mut idxs = Vec::new();
                for n in names {
                    idxs.push(schema.column_index(n).ok_or_else(|| PlanError::NoSuchColumn(n.clone()))?);
                }
                (names.clone(), idxs)
            }
        };

        let mut plan = crate::plan::planner::build_select_plan(&schema, where_clause, indices);
        let mut rows = Vec::new();
        while let Some(row) = plan.next(&mut self.pager)? {
            rows.push(row);
        }
        Ok(ExecResult::Rows { columns: out_names, rows })
    }
```

Note: this Task 28 version of `execute_select` ignores `order_by`/`limit` (bound to `_order_by`/`_limit` in the match arm to silence unused-field warnings without pretending they're supported). Task 31 changes this match arm to bind and use them.

- [ ] **Step 6: Write and run an engine-level SELECT test**

Add to `src/engine.rs`'s `tests` module:

```rust
    #[test]
    fn select_with_where_and_projection() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)").unwrap();
        db.execute("INSERT INTO t (id, name) VALUES (1, 'a'), (2, 'b'), (3, 'c')").unwrap();

        let result = db.execute("SELECT name FROM t WHERE id > 1").unwrap();
        match result {
            ExecResult::Rows { columns, rows } => {
                assert_eq!(columns, vec!["name".to_string()]);
                assert_eq!(rows, vec![vec![Value::Text("b".into())], vec![Value::Text("c".into())]]);
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }
```

Run: `cargo test engine::tests`
Expected: PASS (7 tests)

- [ ] **Step 7: Commit**

```bash
git add src/plan/planner.rs src/engine.rs
git commit -m "Add SELECT execution via sequential scan, filter, and projection"
```

---

## Task 29: End-to-end integration test

**Files:**
- Create: `tests/integration.rs`

**Interfaces:**
- Consumes: the public `dbengine::{Database, ExecResult}` API from Tasks 26–28.
- Produces: nothing new — a `tests/` integration test crate proving CREATE/INSERT/SELECT work together against a real temp file, exercised through the crate's public API rather than internal modules.

- [ ] **Step 1: Write the test**

Write `tests/integration.rs`:

```rust
use dbengine::{Database, ExecResult};
use dbengine::types::value::Value;
use tempfile::NamedTempFile;

#[test]
fn create_insert_select_end_to_end() {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();

    db.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, active BOOLEAN)")
        .unwrap();

    db.execute("INSERT INTO users (id, name, active) VALUES (1, 'Ada', TRUE), (2, 'Bea', FALSE), (3, 'Cy', TRUE)")
        .unwrap();

    let result = db.execute("SELECT name FROM users WHERE active = TRUE ORDER BY id").unwrap();
    match result {
        ExecResult::Rows { columns, rows } => {
            assert_eq!(columns, vec!["name".to_string()]);
            assert_eq!(rows, vec![vec![Value::Text("Ada".into())], vec![Value::Text("Cy".into())]]);
        }
        other => panic!("unexpected result: {other:?}"),
    }
}
```

This test's `ORDER BY id` clause is accepted by the parser but not yet applied by the executor (Task 31 completes that); until then, the row order the query happens to return is the natural table-scan order (primary key order, since the table B+Tree is keyed by `id`), so the assertion above holds either way. Once Task 31 lands, this same assertion remains correct — now because `ORDER BY` is genuinely enforced rather than incidental.

- [ ] **Step 2: Expose `dbengine::types` publicly if not already**

Check `src/lib.rs`: `pub mod types;` was declared in Task 1, so `dbengine::types::value::Value` is already reachable. No change needed unless the module was accidentally left private — if so, add `pub mod types;` to `src/lib.rs`.

- [ ] **Step 3: Run the test**

Run: `cargo test --test integration`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add tests/integration.rs
git commit -m "Add end-to-end integration test for CREATE/INSERT/SELECT"
```

---

## Task 30: TableSeek operator and planner primary-key rule

**Files:**
- Modify: `src/exec/scan.rs` (add `TableSeek` alongside `SeqScan`)
- Modify: `src/plan/planner.rs`

**Interfaces:**
- Consumes: everything from Tasks 25–29.
- Produces: `scan::TableSeek::new(TableSchema, key: Vec<u8>) -> TableSeek` implementing `Operator`. `build_select_plan`'s signature is unchanged (`(&TableSchema, Option<Expr>, Vec<usize>) -> Box<dyn Operator>`), but it now detects a top-level `pk_column = literal` conjunct in `where_clause` and uses `TableSeek` instead of `SeqScan`, wrapping any remaining conjuncts in `Filter`. A top-level `OR` anywhere in the predicate disables this optimization (falls back to `SeqScan` + `Filter` unchanged from Task 28).

- [ ] **Step 1: Write the failing test for `TableSeek`**

Add to the `tests` module in `src/exec/scan.rs`:

```rust
    #[test]
    fn table_seek_finds_single_row_by_primary_key() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let initial_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(initial_root).unwrap());
        let schema = schema_with_root(initial_root);
        let final_root = {
            let mut bt = BTree::new(&mut pager, initial_root);
            for i in [1, 2, 3] {
                let row = vec![Value::Integer(i)];
                bt.insert(&crate::types::value::encode_key(&Value::Integer(i)), &crate::types::row::encode_row(&schema, &row)).unwrap();
            }
            bt.root()
        };

        let key = crate::types::value::encode_key(&Value::Integer(2));
        let mut seek = TableSeek::new(schema_with_root(final_root), key);
        assert_eq!(seek.next(&mut pager).unwrap(), Some(vec![Value::Integer(2)]));
        assert_eq!(seek.next(&mut pager).unwrap(), None, "seek yields at most one row");

        let missing_key = crate::types::value::encode_key(&Value::Integer(99));
        let mut seek_missing = TableSeek::new(schema_with_root(final_root), missing_key);
        assert_eq!(seek_missing.next(&mut pager).unwrap(), None);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test exec::scan::tests::table_seek_finds_single_row_by_primary_key`
Expected: FAIL — `TableSeek` not defined.

- [ ] **Step 3: Implement `TableSeek`**

Add to `src/exec/scan.rs`, above the test module:

```rust
pub struct TableSeek {
    root: u32,
    schema: TableSchema,
    key: Vec<u8>,
    done: bool,
}

impl TableSeek {
    pub fn new(schema: TableSchema, key: Vec<u8>) -> Self {
        let root = schema.root_page;
        TableSeek { root, schema, key, done: false }
    }
}

impl Operator for TableSeek {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Vec<Value>>, ExecError> {
        if self.done {
            return Ok(None);
        }
        self.done = true;
        let mut bt = BTree::new(pager, self.root);
        match bt.search(&self.key)? {
            Some(payload) => Ok(Some(decode_row(&self.schema, &payload))),
            None => Ok(None),
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test exec::scan::tests`
Expected: PASS (2 tests)

- [ ] **Step 5: Write the failing test for the planner's PK rule**

Add to the `tests` module in `src/plan/planner.rs`:

```rust
    #[test]
    fn pk_equality_predicate_uses_table_seek_and_touches_few_pages() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let initial_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(initial_root).unwrap());

        let mut schema = TableSchema {
            name: "t".into(),
            columns: vec![Column { name: "id".into(), ty: ColumnType::Integer, not_null: true, is_primary_key: true }],
            root_page: initial_root,
        };
        let final_root = {
            let mut bt = crate::btree::tree::BTree::new(&mut pager, initial_root);
            for i in 0..5000i64 {
                let row = vec![Value::Integer(i)];
                bt.insert(&crate::types::value::encode_key(&Value::Integer(i)), &crate::types::row::encode_row(&schema, &row)).unwrap();
            }
            bt.root()
        };
        schema.root_page = final_root;
        pager.flush().unwrap();
        // Force a genuinely cold cache before measuring: the pager's LRU cache
        // holds 256 pages, and the tree built above easily fits, so every page
        // touched during setup is still resident. Without reopening, pages_read
        // would report 0 for ANY query afterward (TableSeek or a full SeqScan
        // alike), making the assertion below vacuously true regardless of which
        // operator actually ran -- proving nothing about page efficiency.
        drop(pager);
        let mut pager = Pager::open(file.path()).unwrap();

        let predicate = Expr::BinaryOp {
            op: crate::sql::ast::BinOp::Eq,
            left: Box::new(Expr::Column("id".into())),
            right: Box::new(Expr::IntLiteral(2500)),
        };
        pager.reset_read_counter();
        let mut plan = build_select_plan(&schema, Some(predicate), vec![0]);
        let mut rows = Vec::new();
        while let Some(row) = plan.next(&mut pager).unwrap() {
            rows.push(row);
        }
        assert_eq!(rows, vec![vec![Value::Integer(2500)]]);
        let table_seek_pages = pager.stats().pages_read;
        assert!(
            table_seek_pages < 20,
            "a PK-equality lookup on a 5000-row tree should touch only a handful of pages, touched {table_seek_pages}"
        );

        // Directly prove the optimization's value, not just an absolute threshold:
        // reopen again (cold cache) and run the SAME predicate through a hand-built
        // SeqScan + Filter plan (Task 28's pre-TableSeek behavior), which must walk
        // the entire leaf chain and therefore read strictly more pages.
        drop(pager);
        let mut pager = Pager::open(file.path()).unwrap();
        pager.reset_read_counter();
        let seq_scan: Box<dyn crate::exec::Operator> = Box::new(SeqScan::new(schema.clone()));
        let mut seq_plan = Filter { input: seq_scan, schema: schema.clone(), predicate: predicate_for_seq_scan() };
        let mut seq_rows = Vec::new();
        while let Some(row) = seq_plan.next(&mut pager).unwrap() {
            seq_rows.push(row);
        }
        assert_eq!(seq_rows, vec![vec![Value::Integer(2500)]]);
        let seq_scan_pages = pager.stats().pages_read;
        assert!(
            table_seek_pages < seq_scan_pages,
            "TableSeek ({table_seek_pages} pages) should read strictly fewer pages than a full SeqScan ({seq_scan_pages} pages) for the same predicate"
        );
    }

    fn predicate_for_seq_scan() -> Expr {
        Expr::BinaryOp {
            op: crate::sql::ast::BinOp::Eq,
            left: Box::new(Expr::Column("id".into())),
            right: Box::new(Expr::IntLiteral(2500)),
        }
    }
```

- [ ] **Step 6: Run test to verify it fails**

Run: `cargo test plan::planner::tests::pk_equality_predicate_uses_table_seek_and_touches_few_pages`
Expected: FAIL — before `TableSeek` exists, `build_select_plan` routes everything through `SeqScan`, so this test fails to compile/run against the old planner. Note: an earlier version of this test used a warm pager cache and a bare `pages_read < N` threshold, which is vacuously true regardless of which operator ran (the 256-page LRU cache holds the whole 500-row tree, so `pages_read` reports 0 either way). The corrected version above reopens the pager (cold cache) before each measurement and adds a direct comparative assertion against a hand-built `SeqScan` + `Filter` plan, proving `TableSeek` genuinely reads fewer pages rather than just satisfying an arbitrary threshold.

- [ ] **Step 7: Implement the primary-key routing rule**

Replace `build_select_plan` in `src/plan/planner.rs`:

```rust
use crate::exec::scan::TableSeek;
use crate::sql::ast::BinOp;
use crate::types::value::{encode_key, Value};

pub fn build_select_plan(schema: &TableSchema, where_clause: Option<Expr>, projection_indices: Vec<usize>) -> Box<dyn Operator> {
    let pk_col = schema.columns[schema.primary_key_index()].name.clone();
    let (pk_value, residual) = extract_pk_equality(where_clause, &pk_col);

    let mut plan: Box<dyn Operator> = match pk_value {
        Some(v) => Box::new(TableSeek::new(schema.clone(), encode_key(&v))),
        None => Box::new(SeqScan::new(schema.clone())),
    };
    if let Some(predicate) = residual {
        plan = Box::new(Filter { input: plan, schema: schema.clone(), predicate });
    }
    Box::new(Project { input: plan, indices: projection_indices })
}

fn split_and_conjuncts(expr: Expr, out: &mut Vec<Expr>) {
    match expr {
        Expr::BinaryOp { op: BinOp::And, left, right } => {
            split_and_conjuncts(*left, out);
            split_and_conjuncts(*right, out);
        }
        other => out.push(other),
    }
}

fn rebuild_and(mut conjuncts: Vec<Expr>) -> Option<Expr> {
    if conjuncts.is_empty() {
        return None;
    }
    let mut acc = conjuncts.remove(0);
    for c in conjuncts {
        acc = Expr::BinaryOp { op: BinOp::And, left: Box::new(acc), right: Box::new(c) };
    }
    Some(acc)
}

fn contains_or(expr: &Expr) -> bool {
    match expr {
        Expr::BinaryOp { op: BinOp::Or, .. } => true,
        Expr::BinaryOp { left, right, .. } => contains_or(left) || contains_or(right),
        _ => false,
    }
}

fn literal_value(expr: &Expr) -> Option<Value> {
    match expr {
        Expr::IntLiteral(n) => Some(Value::Integer(*n)),
        Expr::StringLiteral(s) => Some(Value::Text(s.clone())),
        Expr::BoolLiteral(b) => Some(Value::Boolean(*b)),
        _ => None,
    }
}

/// Splits `where_clause` into (an optional PK-equality literal to seek on, the
/// remaining predicate to apply as a residual filter). Only a top-level chain of
/// `AND`s is analyzed; any `OR` anywhere disables the optimization entirely and
/// the whole expression is returned as the residual filter.
fn extract_pk_equality(where_clause: Option<Expr>, pk_col: &str) -> (Option<Value>, Option<Expr>) {
    let Some(expr) = where_clause else {
        return (None, None);
    };
    if contains_or(&expr) {
        return (None, Some(expr));
    }
    let mut conjuncts = Vec::new();
    split_and_conjuncts(expr, &mut conjuncts);

    let mut pk_value = None;
    let mut remaining = Vec::new();
    for c in conjuncts {
        if pk_value.is_none() {
            if let Expr::BinaryOp { op: BinOp::Eq, left, right } = &c {
                let matched = match (left.as_ref(), right.as_ref()) {
                    (Expr::Column(name), lit) if name == pk_col => literal_value(lit),
                    (lit, Expr::Column(name)) if name == pk_col => literal_value(lit),
                    _ => None,
                };
                if let Some(v) = matched {
                    pk_value = Some(v);
                    continue;
                }
            }
        }
        remaining.push(c);
    }
    (pk_value, rebuild_and(remaining))
}
```

- [ ] **Step 8: Run tests to verify they pass**

Run: `cargo test plan::planner::tests`
Expected: PASS (2 tests)

- [ ] **Step 9: Run the full test suite to confirm nothing broke**

Run: `cargo test`
Expected: PASS — all tests through Task 30, including `engine::tests` and `tests/integration.rs`, which exercise `build_select_plan` indirectly and must still pass unchanged.

- [ ] **Step 10: Commit**

```bash
git add src/exec/scan.rs src/plan/planner.rs
git commit -m "Add TableSeek and planner rule to route PK-equality WHERE clauses"
```

---

## Task 31: Sort and Limit operators; ORDER BY / LIMIT wiring

**Files:**
- Create: `src/exec/sort.rs`
- Create: `src/exec/limit.rs`
- Modify: `src/plan/planner.rs`
- Modify: `src/engine.rs`

**Interfaces:**
- Consumes: everything from Task 30, `value::sql_cmp_nullable`.
- Produces: `sort::Sort::new(Box<dyn Operator>, key_index: usize, descending: bool) -> Sort`, `limit::Limit::new(Box<dyn Operator>, n: i64) -> Limit`, both implementing `Operator`. `build_select_plan` gains two parameters and now returns `Result<Box<dyn Operator>, PlanError>`: `build_select_plan(&TableSchema, Option<Expr>, Vec<usize>, Option<(String, bool)>, Option<i64>) -> Result<Box<dyn Operator>, PlanError>`. Plan shape is scan/seek → filter → **sort (on the full pre-projection row)** → project → limit — sorting before projecting matters because `ORDER BY` may reference a column that isn't in the `SELECT` list.

- [ ] **Step 1: Write the failing test for `Sort`**

Write `src/exec/sort.rs`:

```rust
use crate::error::ExecError;
use crate::exec::Operator;
use crate::storage::pager::Pager;
use crate::types::value::{sql_cmp_nullable, Value};

pub struct Sort {
    input: Box<dyn Operator>,
    key_index: usize,
    descending: bool,
    buffer: Option<std::vec::IntoIter<Vec<Value>>>,
}

impl Sort {
    pub fn new(input: Box<dyn Operator>, key_index: usize, descending: bool) -> Self {
        Sort { input, key_index, descending, buffer: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    struct Fixed(std::vec::IntoIter<Vec<Value>>);
    impl Operator for Fixed {
        fn next(&mut self, _pager: &mut Pager) -> Result<Option<Vec<Value>>, ExecError> {
            Ok(self.0.next())
        }
    }

    fn pager() -> Pager {
        let file = NamedTempFile::new().unwrap();
        Pager::create(file.path()).unwrap()
    }

    #[test]
    fn sorts_ascending_by_key_index() {
        let input = Fixed(vec![vec![Value::Integer(3)], vec![Value::Integer(1)], vec![Value::Integer(2)]].into_iter());
        let mut sort = Sort::new(Box::new(input), 0, false);
        let mut pager = pager();
        let mut seen = Vec::new();
        while let Some(row) = sort.next(&mut pager).unwrap() {
            seen.push(row[0].clone());
        }
        assert_eq!(seen, vec![Value::Integer(1), Value::Integer(2), Value::Integer(3)]);
    }

    #[test]
    fn sorts_descending_and_places_nulls_first() {
        let input = Fixed(vec![vec![Value::Integer(1)], vec![Value::Null], vec![Value::Integer(2)]].into_iter());
        let mut sort = Sort::new(Box::new(input), 0, true);
        let mut pager = pager();
        let mut seen = Vec::new();
        while let Some(row) = sort.next(&mut pager).unwrap() {
            seen.push(row[0].clone());
        }
        // descending reverses the whole comparator, including the Null-sorts-first rule,
        // so Null (normally first) ends up last under DESC.
        assert_eq!(seen, vec![Value::Integer(2), Value::Integer(1), Value::Null]);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test exec::sort::tests`
Expected: FAIL — `Operator` not implemented for `Sort`.

- [ ] **Step 3: Implement `Sort`**

Add to `src/exec/sort.rs`, above the test module:

```rust
impl Operator for Sort {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Vec<Value>>, ExecError> {
        if self.buffer.is_none() {
            let mut rows = Vec::new();
            while let Some(r) = self.input.next(pager)? {
                rows.push(r);
            }
            rows.sort_by(|a, b| {
                let ord = sql_cmp_nullable(&a[self.key_index], &b[self.key_index]);
                if self.descending { ord.reverse() } else { ord }
            });
            self.buffer = Some(rows.into_iter());
        }
        Ok(self.buffer.as_mut().unwrap().next())
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test exec::sort::tests`
Expected: PASS (2 tests)

- [ ] **Step 5: Write and implement `Limit`**

Write `src/exec/limit.rs`:

```rust
use crate::error::ExecError;
use crate::exec::Operator;
use crate::storage::pager::Pager;
use crate::types::value::Value;

pub struct Limit {
    input: Box<dyn Operator>,
    remaining: i64,
}

impl Limit {
    pub fn new(input: Box<dyn Operator>, n: i64) -> Self {
        Limit { input, remaining: n }
    }
}

impl Operator for Limit {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Vec<Value>>, ExecError> {
        if self.remaining <= 0 {
            return Ok(None);
        }
        self.remaining -= 1;
        self.input.next(pager)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    struct Fixed(std::vec::IntoIter<Vec<Value>>);
    impl Operator for Fixed {
        fn next(&mut self, _pager: &mut Pager) -> Result<Option<Vec<Value>>, ExecError> {
            Ok(self.0.next())
        }
    }

    #[test]
    fn stops_after_n_rows() {
        let input = Fixed(vec![vec![Value::Integer(1)], vec![Value::Integer(2)], vec![Value::Integer(3)]].into_iter());
        let mut limit = Limit::new(Box::new(input), 2);
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let mut seen = Vec::new();
        while let Some(row) = limit.next(&mut pager).unwrap() {
            seen.push(row[0].clone());
        }
        assert_eq!(seen, vec![Value::Integer(1), Value::Integer(2)]);
    }

    #[test]
    fn zero_limit_yields_nothing() {
        let input = Fixed(vec![vec![Value::Integer(1)]].into_iter());
        let mut limit = Limit::new(Box::new(input), 0);
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        assert_eq!(limit.next(&mut pager).unwrap(), None);
    }
}
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo test exec::limit::tests`
Expected: PASS (2 tests)

- [ ] **Step 7: Wire `Sort`/`Limit` into the planner**

In `src/plan/planner.rs`, change `build_select_plan`'s signature and body:

```rust
use crate::error::PlanError;
use crate::exec::limit::Limit;
use crate::exec::sort::Sort;

pub fn build_select_plan(
    schema: &TableSchema,
    where_clause: Option<Expr>,
    projection_indices: Vec<usize>,
    order_by: Option<(String, bool)>,
    limit: Option<i64>,
) -> Result<Box<dyn Operator>, PlanError> {
    let pk_col = schema.columns[schema.primary_key_index()].name.clone();
    let (pk_value, residual) = extract_pk_equality(where_clause, &pk_col);

    let mut plan: Box<dyn Operator> = match pk_value {
        Some(v) => Box::new(TableSeek::new(schema.clone(), encode_key(&v))),
        None => Box::new(SeqScan::new(schema.clone())),
    };
    if let Some(predicate) = residual {
        plan = Box::new(Filter { input: plan, schema: schema.clone(), predicate });
    }
    if let Some((col, desc)) = order_by {
        let idx = schema.column_index(&col).ok_or_else(|| PlanError::NoSuchColumn(col))?;
        plan = Box::new(Sort::new(plan, idx, desc));
    }
    plan = Box::new(Project { input: plan, indices: projection_indices });
    if let Some(n) = limit {
        plan = Box::new(Limit::new(plan, n));
    }
    Ok(plan)
}
```

Update the Task 28 test (`builds_a_plan_that_scans_filters_and_projects`) and the Task 30 test (`pk_equality_predicate_uses_table_seek_and_touches_few_pages`) in this same file: both call `build_select_plan(&schema, predicate_or_none, indices)` with 3 arguments. Change both call sites to `build_select_plan(&schema, predicate_or_none, indices, None, None).unwrap()`.

- [ ] **Step 8: Update the engine to use the new signature**

In `src/engine.rs`, change the `Select` match arm in `execute` back to binding `order_by`/`limit` instead of discarding them:

```rust
            Statement::Select { columns, table, where_clause, order_by, limit } => {
                self.execute_select(columns, &table, where_clause, order_by, limit)?
            }
```

Change `execute_select`'s signature and body:

```rust
    fn execute_select(
        &mut self,
        columns: SelectColumns,
        table: &str,
        where_clause: Option<Expr>,
        order_by: Option<(String, bool)>,
        limit: Option<i64>,
    ) -> Result<ExecResult> {
        let schema = self
            .catalog
            .get_table(&mut self.pager, table)?
            .ok_or_else(|| PlanError::NoSuchTable(table.to_string()))?;

        let (out_names, indices): (Vec<String>, Vec<usize>) = match &columns {
            SelectColumns::All => (
                schema.columns.iter().map(|c| c.name.clone()).collect(),
                (0..schema.columns.len()).collect(),
            ),
            SelectColumns::List(names) => {
                let mut idxs = Vec::new();
                for n in names {
                    idxs.push(schema.column_index(n).ok_or_else(|| PlanError::NoSuchColumn(n.clone()))?);
                }
                (names.clone(), idxs)
            }
        };

        let mut plan = crate::plan::planner::build_select_plan(&schema, where_clause, indices, order_by, limit)?;
        let mut rows = Vec::new();
        while let Some(row) = plan.next(&mut self.pager)? {
            rows.push(row);
        }
        Ok(ExecResult::Rows { columns: out_names, rows })
    }
```

- [ ] **Step 9: Write an engine-level ORDER BY / LIMIT test**

Add to `src/engine.rs`'s `tests` module:

```rust
    #[test]
    fn select_with_order_by_and_limit() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, score INTEGER)").unwrap();
        db.execute("INSERT INTO t (id, score) VALUES (1, 30), (2, 10), (3, 20)").unwrap();

        let result = db.execute("SELECT id FROM t ORDER BY score LIMIT 2").unwrap();
        match result {
            ExecResult::Rows { rows, .. } => {
                assert_eq!(rows, vec![vec![Value::Integer(2)], vec![Value::Integer(3)]]);
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }
```

- [ ] **Step 10: Run the full test suite**

Run: `cargo test`
Expected: PASS — all tests through Task 31.

- [ ] **Step 11: Commit**

```bash
git add src/exec/sort.rs src/exec/limit.rs src/plan/planner.rs src/engine.rs
git commit -m "Add Sort and Limit operators with ORDER BY / LIMIT wiring"
```

---

## Task 32: DELETE execution

**Files:**
- Modify: `src/exec/mutate.rs`
- Modify: `src/engine.rs`

**Interfaces:**
- Consumes: everything from Task 31.
- Produces: `mutate::delete_row(&mut Pager, &TableSchema, &[IndexSchema], &[Value]) -> Result<(u32, Vec<(String, u32)>), ExecError>` — same "return new roots" contract as `insert_row`. `Database::execute` gains the `Delete` match arm.

Design note: per the Global Constraints, mutating a B+Tree invalidates any cursor walking it. `execute_delete` therefore fully collects every matching row (via the existing `build_select_plan`, run to completion) *before* deleting any of them — never deletes while a scan cursor is still live over the same tree.

- [ ] **Step 1: Write the failing test for `delete_row`**

Add to the `tests` module in `src/exec/mutate.rs`:

```rust
    #[test]
    fn deletes_row_and_its_index_entries() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let table_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(table_root).unwrap());
        let index_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(index_root).unwrap());
        let s = schema(table_root);
        let idx = IndexSchema { name: "idx_name".into(), table: "t".into(), column: "name".into(), root_page: index_root };

        let (table_root, roots) = insert_row(&mut pager, &s, &[idx.clone()], &[Value::Integer(1), Value::Text("a".into())]).unwrap();
        let mut s2 = s.clone();
        s2.root_page = table_root;
        let mut idx2 = idx.clone();
        idx2.root_page = roots[0].1;

        let (new_table_root, new_index_roots) =
            delete_row(&mut pager, &s2, &[idx2.clone()], &[Value::Integer(1), Value::Text("a".into())]).unwrap();

        let mut bt = BTree::new(&mut pager, new_table_root);
        assert_eq!(bt.search(&crate::types::value::encode_key(&Value::Integer(1))).unwrap(), None);

        let mut ibt = BTree::new(&mut pager, new_index_roots[0].1);
        let idx_key = crate::types::value::encode_composite_key(&[Value::Text("a".into()), Value::Integer(1)]);
        assert_eq!(ibt.search(&idx_key).unwrap(), None);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test exec::mutate::tests::deletes_row_and_its_index_entries`
Expected: FAIL — `delete_row` not defined.

- [ ] **Step 3: Implement `delete_row`**

Add to `src/exec/mutate.rs`, below `insert_row`:

```rust
pub fn delete_row(
    pager: &mut Pager,
    schema: &TableSchema,
    indexes: &[IndexSchema],
    row: &[Value],
) -> Result<(u32, Vec<(String, u32)>), ExecError> {
    let pk_idx = schema.primary_key_index();
    let key = encode_key(&row[pk_idx]);
    let mut bt = BTree::new(pager, schema.root_page);
    bt.delete(&key)?;
    let new_table_root = bt.root();

    let mut new_index_roots = Vec::new();
    for idx in indexes {
        let col_idx = schema.column_index(&idx.column).expect("index column must exist in table schema");
        let idx_key = encode_composite_key(&[row[col_idx].clone(), row[pk_idx].clone()]);
        let mut ibt = BTree::new(pager, idx.root_page);
        ibt.delete(&idx_key)?;
        new_index_roots.push((idx.name.clone(), ibt.root()));
    }
    Ok((new_table_root, new_index_roots))
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test exec::mutate::tests`
Expected: PASS (5 tests)

- [ ] **Step 5: Wire `Delete` into the engine**

Add `Statement::Delete { table, where_clause } => self.execute_delete(&table, where_clause)?,` as a new match arm in `src/engine.rs`'s `execute`.

Add to `impl Database`:

```rust
    fn execute_delete(&mut self, table: &str, where_clause: Option<Expr>) -> Result<ExecResult> {
        let schema = self
            .catalog
            .get_table(&mut self.pager, table)?
            .ok_or_else(|| PlanError::NoSuchTable(table.to_string()))?;
        let indexes = self.catalog.list_indexes_for_table(&mut self.pager, table)?;

        let all_columns: Vec<usize> = (0..schema.columns.len()).collect();
        let mut plan = crate::plan::planner::build_select_plan(&schema, where_clause, all_columns, None, None)?;
        let mut rows_to_delete = Vec::new();
        while let Some(row) = plan.next(&mut self.pager)? {
            rows_to_delete.push(row);
        }

        let mut table_root = schema.root_page;
        let mut index_roots: HashMap<String, u32> =
            indexes.iter().map(|i| (i.name.clone(), i.root_page)).collect();
        let mut count = 0usize;

        for row in &rows_to_delete {
            let mut schema_for_write = schema.clone();
            schema_for_write.root_page = table_root;
            let indexes_for_write: Vec<IndexSchema> = indexes
                .iter()
                .cloned()
                .map(|mut idx| {
                    idx.root_page = index_roots[&idx.name];
                    idx
                })
                .collect();

            let (new_table_root, new_index_roots) =
                crate::exec::mutate::delete_row(&mut self.pager, &schema_for_write, &indexes_for_write, row)?;
            table_root = new_table_root;
            for (name, root) in new_index_roots {
                index_roots.insert(name, root);
            }
            count += 1;
        }

        if table_root != schema.root_page {
            self.catalog.update_table_root(&mut self.pager, table, table_root)?;
        }
        for idx in &indexes {
            let new_root = index_roots[&idx.name];
            if new_root != idx.root_page {
                self.catalog.update_index_root(&mut self.pager, &idx.name, new_root)?;
            }
        }

        Ok(ExecResult::Modified(count))
    }
```

- [ ] **Step 6: Write and run an engine-level DELETE test**

Add to `src/engine.rs`'s `tests` module:

```rust
    #[test]
    fn delete_removes_matching_rows_only() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)").unwrap();
        db.execute("INSERT INTO t (id) VALUES (1), (2), (3)").unwrap();

        assert_eq!(db.execute("DELETE FROM t WHERE id = 2").unwrap(), ExecResult::Modified(1));

        let result = db.execute("SELECT id FROM t").unwrap();
        match result {
            ExecResult::Rows { rows, .. } => {
                let mut remaining: Vec<i64> = rows.iter().map(|r| match &r[0] { Value::Integer(n) => *n, _ => unreachable!() }).collect();
                remaining.sort();
                assert_eq!(remaining, vec![1, 3]);
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }
```

Run: `cargo test engine::tests`
Expected: PASS (9 tests)

- [ ] **Step 7: Commit**

```bash
git add src/exec/mutate.rs src/engine.rs
git commit -m "Add DELETE execution with collect-then-mutate row handling"
```

---

## Task 33: UPDATE execution

**Files:**
- Modify: `src/exec/mutate.rs`
- Modify: `src/engine.rs`

**Interfaces:**
- Consumes: everything from Task 32, `plan::expr::eval`.
- Produces: `mutate::update_row(&mut Pager, &TableSchema, &[IndexSchema], old_row: &[Value], new_row: &[Value]) -> Result<(u32, Vec<(String, u32)>), ExecError>`, implemented by composing `delete_row` followed by `insert_row` — this reuses both existing primitives (including their NOT NULL and duplicate-PK checks) rather than duplicating that logic. `Database::execute` gains the `Update` match arm.

- [ ] **Step 1: Write the failing test for `update_row`**

Add to the `tests` module in `src/exec/mutate.rs`:

```rust
    #[test]
    fn updates_row_and_reindexes_changed_column() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let table_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(table_root).unwrap());
        let index_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(index_root).unwrap());
        let s = schema(table_root);
        let idx = IndexSchema { name: "idx_name".into(), table: "t".into(), column: "name".into(), root_page: index_root };

        let (table_root, roots) = insert_row(&mut pager, &s, &[idx.clone()], &[Value::Integer(1), Value::Text("a".into())]).unwrap();
        let mut s2 = s.clone();
        s2.root_page = table_root;
        let mut idx2 = idx.clone();
        idx2.root_page = roots[0].1;

        let old_row = vec![Value::Integer(1), Value::Text("a".into())];
        let new_row = vec![Value::Integer(1), Value::Text("z".into())];
        let (new_table_root, new_index_roots) =
            update_row(&mut pager, &s2, &[idx2.clone()], &old_row, &new_row).unwrap();

        let mut bt = BTree::new(&mut pager, new_table_root);
        let payload = bt.search(&crate::types::value::encode_key(&Value::Integer(1))).unwrap().unwrap();
        assert_eq!(crate::types::row::decode_row(&s2, &payload), new_row);

        let mut ibt = BTree::new(&mut pager, new_index_roots[0].1);
        let old_key = crate::types::value::encode_composite_key(&[Value::Text("a".into()), Value::Integer(1)]);
        let new_key = crate::types::value::encode_composite_key(&[Value::Text("z".into()), Value::Integer(1)]);
        assert_eq!(ibt.search(&old_key).unwrap(), None);
        assert_eq!(ibt.search(&new_key).unwrap(), Some(vec![]));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test exec::mutate::tests::updates_row_and_reindexes_changed_column`
Expected: FAIL — `update_row` not defined.

- [ ] **Step 3: Implement `update_row`**

Add to `src/exec/mutate.rs`, below `delete_row`:

```rust
pub fn update_row(
    pager: &mut Pager,
    schema: &TableSchema,
    indexes: &[IndexSchema],
    old_row: &[Value],
    new_row: &[Value],
) -> Result<(u32, Vec<(String, u32)>), ExecError> {
    let (table_root_after_delete, index_roots_after_delete) = delete_row(pager, schema, indexes, old_row)?;

    let mut schema_after_delete = schema.clone();
    schema_after_delete.root_page = table_root_after_delete;
    let indexes_after_delete: Vec<IndexSchema> = indexes
        .iter()
        .cloned()
        .map(|mut idx| {
            if let Some((_, root)) = index_roots_after_delete.iter().find(|(name, _)| name == &idx.name) {
                idx.root_page = *root;
            }
            idx
        })
        .collect();

    insert_row(pager, &schema_after_delete, &indexes_after_delete, new_row)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test exec::mutate::tests`
Expected: PASS (6 tests)

- [ ] **Step 5: Wire `Update` into the engine**

Add `Statement::Update { table, assignments, where_clause } => self.execute_update(&table, assignments, where_clause)?,` as a new match arm in `src/engine.rs`'s `execute`.

Add to `impl Database`:

```rust
    fn execute_update(
        &mut self,
        table: &str,
        assignments: Vec<(String, Expr)>,
        where_clause: Option<Expr>,
    ) -> Result<ExecResult> {
        let schema = self
            .catalog
            .get_table(&mut self.pager, table)?
            .ok_or_else(|| PlanError::NoSuchTable(table.to_string()))?;
        let indexes = self.catalog.list_indexes_for_table(&mut self.pager, table)?;

        let mut assignment_indices = Vec::new();
        for (name, expr) in assignments {
            let idx = schema.column_index(&name).ok_or_else(|| PlanError::NoSuchColumn(name.clone()))?;
            assignment_indices.push((idx, expr));
        }

        let all_columns: Vec<usize> = (0..schema.columns.len()).collect();
        let mut plan = crate::plan::planner::build_select_plan(&schema, where_clause, all_columns, None, None)?;
        let mut old_rows = Vec::new();
        while let Some(row) = plan.next(&mut self.pager)? {
            old_rows.push(row);
        }

        let mut table_root = schema.root_page;
        let mut index_roots: HashMap<String, u32> =
            indexes.iter().map(|i| (i.name.clone(), i.root_page)).collect();
        let mut count = 0usize;

        for old_row in &old_rows {
            let mut new_row = old_row.clone();
            for (idx, expr) in &assignment_indices {
                new_row[*idx] =
                    crate::plan::expr::eval(expr, &schema, old_row).map_err(DbError::Plan)?;
            }

            let mut schema_for_write = schema.clone();
            schema_for_write.root_page = table_root;
            let indexes_for_write: Vec<IndexSchema> = indexes
                .iter()
                .cloned()
                .map(|mut idx| {
                    idx.root_page = index_roots[&idx.name];
                    idx
                })
                .collect();

            let (new_table_root, new_index_roots) = crate::exec::mutate::update_row(
                &mut self.pager,
                &schema_for_write,
                &indexes_for_write,
                old_row,
                &new_row,
            )?;
            table_root = new_table_root;
            for (name, root) in new_index_roots {
                index_roots.insert(name, root);
            }
            count += 1;
        }

        if table_root != schema.root_page {
            self.catalog.update_table_root(&mut self.pager, table, table_root)?;
        }
        for idx in &indexes {
            let new_root = index_roots[&idx.name];
            if new_root != idx.root_page {
                self.catalog.update_index_root(&mut self.pager, &idx.name, new_root)?;
            }
        }

        Ok(ExecResult::Modified(count))
    }
```

- [ ] **Step 6: Write and run an engine-level UPDATE test**

Add to `src/engine.rs`'s `tests` module:

```rust
    #[test]
    fn update_changes_matching_rows_only() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)").unwrap();
        db.execute("INSERT INTO t (id, name) VALUES (1, 'a'), (2, 'b')").unwrap();

        assert_eq!(db.execute("UPDATE t SET name = 'z' WHERE id = 1").unwrap(), ExecResult::Modified(1));

        let result = db.execute("SELECT id, name FROM t WHERE id = 1").unwrap();
        match result {
            ExecResult::Rows { rows, .. } => {
                assert_eq!(rows, vec![vec![Value::Integer(1), Value::Text("z".into())]]);
            }
            other => panic!("unexpected result: {other:?}"),
        }
        let unchanged = db.execute("SELECT name FROM t WHERE id = 2").unwrap();
        match unchanged {
            ExecResult::Rows { rows, .. } => assert_eq!(rows, vec![vec![Value::Text("b".into())]]),
            other => panic!("unexpected result: {other:?}"),
        }
    }
```

Run: `cargo test engine::tests`
Expected: PASS (10 tests)

- [ ] **Step 7: Run the full test suite**

Run: `cargo test`
Expected: PASS — all tests through Task 33.

- [ ] **Step 8: Commit**

```bash
git add src/exec/mutate.rs src/engine.rs
git commit -m "Add UPDATE execution by composing delete_row and insert_row"
```

---

## Task 34: CREATE INDEX (build from existing rows) and DROP INDEX

**Files:**
- Modify: `src/engine.rs`

**Interfaces:**
- Consumes: everything from Task 33, `exec::scan::SeqScan`.
- Produces: `Database::execute` gains the `CreateIndex` and `DropIndex` match arms.

- [ ] **Step 1: Write the failing test**

Add to `src/engine.rs`'s `tests` module:

```rust
    #[test]
    fn create_index_on_existing_rows_then_drop_it() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT NOT NULL)").unwrap();
        db.execute("INSERT INTO t (id, name) VALUES (1, 'a'), (2, 'b')").unwrap();

        assert_eq!(db.execute("CREATE INDEX idx_name ON t (name)").unwrap(), ExecResult::Ok);
        assert_eq!(db.execute("DROP INDEX idx_name").unwrap(), ExecResult::Ok);
    }

    #[test]
    fn create_index_on_nullable_column_errors() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)").unwrap();
        let err = db.execute("CREATE INDEX idx_name ON t (name)").unwrap_err();
        assert!(matches!(err, DbError::Plan(PlanError::InvalidSchema(_))));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test engine::tests::create_index_on_existing_rows_then_drop_it`
Expected: FAIL — `CreateIndex`/`DropIndex` statements not yet handled (they currently fall into the catch-all `other => Err(...)` arm from Task 26).

- [ ] **Step 3: Implement**

Add `Statement::CreateIndex { name, table, column } => self.execute_create_index(&name, &table, &column)?,` and `Statement::DropIndex { name } => self.execute_drop_index(&name)?,` as new match arms in `execute`.

Add to `impl Database`:

```rust
    fn execute_create_index(&mut self, name: &str, table: &str, column: &str) -> Result<ExecResult> {
        let schema = self
            .catalog
            .get_table(&mut self.pager, table)?
            .ok_or_else(|| PlanError::NoSuchTable(table.to_string()))?;
        let col_idx = schema
            .column_index(column)
            .ok_or_else(|| PlanError::NoSuchColumn(column.to_string()))?;
        if !schema.columns[col_idx].not_null {
            return Err(DbError::Plan(PlanError::InvalidSchema(format!(
                "indexed column {column} must be NOT NULL"
            ))));
        }

        let initial_index_root = self.pager.allocate_page()?;
        LeafNode { entries: vec![], next_leaf: 0 }.encode(self.pager.get_page_mut(initial_index_root)?);

        let pk_idx = schema.primary_key_index();
        let mut scan = crate::exec::scan::SeqScan::new(schema.clone());
        let mut current_root = initial_index_root;
        while let Some(row) = scan.next(&mut self.pager)? {
            let idx_key = crate::types::value::encode_composite_key(&[row[col_idx].clone(), row[pk_idx].clone()]);
            let mut ibt = crate::btree::tree::BTree::new(&mut self.pager, current_root);
            ibt.insert(&idx_key, &[])?;
            current_root = ibt.root();
        }

        let idx_schema = crate::types::schema::IndexSchema {
            name: name.to_string(),
            table: table.to_string(),
            column: column.to_string(),
            root_page: current_root,
        };
        self.catalog.create_index(&mut self.pager, &idx_schema)?;
        Ok(ExecResult::Ok)
    }

    fn execute_drop_index(&mut self, name: &str) -> Result<ExecResult> {
        self.catalog.drop_index(&mut self.pager, name)?;
        Ok(ExecResult::Ok)
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test engine::tests`
Expected: PASS (12 tests)

- [ ] **Step 5: Commit**

```bash
git add src/engine.rs
git commit -m "Add CREATE INDEX (built from a full table scan) and DROP INDEX"
```

---

## Task 35: IndexSeek operator and planner index-equality rule

**Files:**
- Modify: `src/exec/scan.rs`
- Modify: `src/plan/planner.rs`
- Modify: `src/engine.rs`

**Interfaces:**
- Consumes: everything from Task 34.
- Produces: `scan::IndexSeek::new(TableSchema, index_root: u32, prefix: Vec<u8>) -> IndexSeek` implementing `Operator`. `build_select_plan` gains an `indexes: &[IndexSchema]` parameter (inserted right after `schema`) and, when no PK-equality match is found, looks for a top-level `indexed_column = literal` conjunct and routes through `IndexSeek` before falling back to `SeqScan`. Every existing caller of `build_select_plan` (Task 28's and Task 30's tests, and `execute_select`/`execute_delete`/`execute_update` in `engine.rs`) must be updated to pass the new parameter.

- [ ] **Step 1: Write the failing test for `IndexSeek`**

Add to the `tests` module in `src/exec/scan.rs`:

```rust
    #[test]
    fn index_seek_finds_row_by_indexed_value() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let table_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(table_root).unwrap());
        let index_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(index_root).unwrap());

        let schema = TableSchema {
            name: "t".into(),
            columns: vec![
                Column { name: "id".into(), ty: ColumnType::Integer, not_null: true, is_primary_key: true },
                Column { name: "name".into(), ty: ColumnType::Text, not_null: true, is_primary_key: false },
            ],
            root_page: table_root,
        };

        let final_table_root;
        let final_index_root;
        {
            let mut tbt = BTree::new(&mut pager, table_root);
            let mut ibt_root = index_root;
            for (id, name) in [(1, "a"), (2, "b"), (3, "a")] {
                let row = vec![Value::Integer(id), Value::Text(name.into())];
                tbt.insert(&crate::types::value::encode_key(&Value::Integer(id)), &crate::types::row::encode_row(&schema, &row)).unwrap();
                let idx_key = crate::types::value::encode_composite_key(&[Value::Text(name.into()), Value::Integer(id)]);
                let mut ibt = BTree::new(pager_ptr(&mut pager), ibt_root);
                let _ = &mut ibt; // placeholder to be replaced below
            }
            final_table_root = tbt.root();
            final_index_root = ibt_root;
        }
        let _ = (final_table_root, final_index_root);
    }
```

The draft above has an aliasing problem (`tbt` and `ibt` both trying to borrow `pager` at once) and a bogus `pager_ptr` call. Write it correctly instead — insert into the table and index trees in **separate, sequential blocks**, not interleaved within one loop body that holds both `BTree` borrows simultaneously:

```rust
    #[test]
    fn index_seek_finds_row_by_indexed_value() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let table_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(table_root).unwrap());
        let index_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(index_root).unwrap());

        let schema = TableSchema {
            name: "t".into(),
            columns: vec![
                Column { name: "id".into(), ty: ColumnType::Integer, not_null: true, is_primary_key: true },
                Column { name: "name".into(), ty: ColumnType::Text, not_null: true, is_primary_key: false },
            ],
            root_page: table_root,
        };

        let rows = [(1, "a"), (2, "b"), (3, "a")];

        let final_table_root = {
            let mut tbt = BTree::new(&mut pager, table_root);
            for (id, name) in rows {
                let row = vec![Value::Integer(id), Value::Text(name.into())];
                tbt.insert(&crate::types::value::encode_key(&Value::Integer(id)), &crate::types::row::encode_row(&schema, &row)).unwrap();
            }
            tbt.root()
        };

        let final_index_root = {
            let mut ibt = BTree::new(&mut pager, index_root);
            for (id, name) in rows {
                let idx_key = crate::types::value::encode_composite_key(&[Value::Text(name.into()), Value::Integer(id)]);
                ibt.insert(&idx_key, &[]).unwrap();
            }
            ibt.root()
        };

        let mut final_schema = schema.clone();
        final_schema.root_page = final_table_root;
        let prefix = crate::types::value::encode_key(&Value::Text("a".into()));
        let mut seek = IndexSeek::new(final_schema, final_index_root, prefix);

        let mut seen = Vec::new();
        while let Some(row) = seek.next(&mut pager).unwrap() {
            seen.push(row[0].clone());
        }
        seen.sort_by_key(|v| match v { Value::Integer(n) => *n, _ => unreachable!() });
        assert_eq!(seen, vec![Value::Integer(1), Value::Integer(3)]);
    }
```

Both table inserts and index inserts happen in their own block, each ending before the next begins — the same non-overlapping-borrow pattern used throughout Tasks 13, 16, and 25.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test exec::scan::tests::index_seek_finds_row_by_indexed_value`
Expected: FAIL — `IndexSeek` not defined.

- [ ] **Step 3: Implement `IndexSeek`**

Add to `src/exec/scan.rs`:

```rust
pub struct IndexSeek {
    index_root: u32,
    schema: TableSchema,
    prefix: Vec<u8>,
    cursor: Cursor,
    started: bool,
}

impl IndexSeek {
    pub fn new(schema: TableSchema, index_root: u32, prefix: Vec<u8>) -> Self {
        IndexSeek { index_root, schema, prefix, cursor: Cursor::empty(), started: false }
    }
}

impl Operator for IndexSeek {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Vec<Value>>, ExecError> {
        if !self.started {
            self.cursor = { BTree::new(pager, self.index_root).cursor_seek(&self.prefix)? };
            self.started = true;
        }
        loop {
            match self.cursor.next(pager)? {
                Some((key, _)) => {
                    if !key.starts_with(self.prefix.as_slice()) {
                        return Ok(None);
                    }
                    let pk_bytes = key[self.prefix.len()..].to_vec();
                    let mut table_bt = BTree::new(pager, self.schema.root_page);
                    if let Some(payload) = table_bt.search(&pk_bytes)? {
                        return Ok(Some(decode_row(&self.schema, &payload)));
                    }
                }
                None => return Ok(None),
            }
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test exec::scan::tests`
Expected: PASS (3 tests)

- [ ] **Step 5: Update the planner**

In `src/plan/planner.rs`, add `use crate::types::schema::IndexSchema;` and `use crate::exec::scan::IndexSeek;`. Change `build_select_plan`'s signature and body:

```rust
pub fn build_select_plan(
    schema: &TableSchema,
    indexes: &[IndexSchema],
    where_clause: Option<Expr>,
    projection_indices: Vec<usize>,
    order_by: Option<(String, bool)>,
    limit: Option<i64>,
) -> Result<Box<dyn Operator>, PlanError> {
    let pk_col = schema.columns[schema.primary_key_index()].name.clone();
    let (pk_value, residual) = extract_pk_equality(where_clause, &pk_col);

    let (mut plan, residual): (Box<dyn Operator>, Option<Expr>) = if let Some(v) = pk_value {
        (Box::new(TableSeek::new(schema.clone(), encode_key(&v))), residual)
    } else if let Some((idx_schema, value, residual2)) = find_index_equality(residual.clone(), indexes) {
        let prefix = encode_key(&value);
        (Box::new(IndexSeek::new(schema.clone(), idx_schema.root_page, prefix)), residual2)
    } else {
        (Box::new(SeqScan::new(schema.clone())), residual)
    };

    if let Some(predicate) = residual {
        plan = Box::new(Filter { input: plan, schema: schema.clone(), predicate });
    }
    if let Some((col, desc)) = order_by {
        let idx = schema.column_index(&col).ok_or_else(|| PlanError::NoSuchColumn(col))?;
        plan = Box::new(Sort::new(plan, idx, desc));
    }
    plan = Box::new(Project { input: plan, indices: projection_indices });
    if let Some(n) = limit {
        plan = Box::new(Limit::new(plan, n));
    }
    Ok(plan)
}

fn find_index_equality(where_clause: Option<Expr>, indexes: &[IndexSchema]) -> Option<(IndexSchema, Value, Option<Expr>)> {
    let expr = where_clause?;
    if contains_or(&expr) {
        return None;
    }
    let mut conjuncts = Vec::new();
    split_and_conjuncts(expr, &mut conjuncts);

    let mut matched: Option<(IndexSchema, Value)> = None;
    let mut remaining = Vec::new();
    for c in conjuncts {
        if matched.is_none() {
            if let Expr::BinaryOp { op: BinOp::Eq, left, right } = &c {
                let candidate = match (left.as_ref(), right.as_ref()) {
                    (Expr::Column(name), lit) => indexes
                        .iter()
                        .find(|i| &i.column == name)
                        .and_then(|i| literal_value(lit).map(|v| (i.clone(), v))),
                    (lit, Expr::Column(name)) => indexes
                        .iter()
                        .find(|i| &i.column == name)
                        .and_then(|i| literal_value(lit).map(|v| (i.clone(), v))),
                    _ => None,
                };
                if let Some(pair) = candidate {
                    matched = Some(pair);
                    continue;
                }
            }
        }
        remaining.push(c);
    }
    matched.map(|(idx, v)| (idx, v, rebuild_and(remaining)))
}
```

- [ ] **Step 6: Update every existing `build_select_plan` call site**

In `src/plan/planner.rs`'s own `tests` module: both `builds_a_plan_that_scans_filters_and_projects` (Task 28) and `pk_equality_predicate_uses_table_seek_and_touches_few_pages` (Task 30) call `build_select_plan(&schema, predicate, indices, None, None)`. Insert `&[]` (an empty index list) as the second argument in both: `build_select_plan(&schema, &[], predicate, indices, None, None).unwrap()`.

In `src/engine.rs`:
- `execute_select`: change `crate::plan::planner::build_select_plan(&schema, where_clause, indices, order_by, limit)?` to `crate::plan::planner::build_select_plan(&schema, &indexes, where_clause, indices, order_by, limit)?` (the `indexes` variable, from `self.catalog.list_indexes_for_table`, already exists earlier in that function — Task 34 didn't need it there yet, so add the line `let indexes = self.catalog.list_indexes_for_table(&mut self.pager, table)?;` right after fetching `schema` if it isn't already present).
- `execute_delete`: change the `build_select_plan(&schema, where_clause, all_columns, None, None)?` call to `build_select_plan(&schema, &indexes, where_clause, all_columns, None, None)?` (the `indexes` variable already exists in this function from Task 32).
- `execute_update`: same change as `execute_delete` (the `indexes` variable already exists from Task 33).

- [ ] **Step 7: Write an engine-level index-seek test**

Add to `src/engine.rs`'s `tests` module:

```rust
    #[test]
    fn select_on_indexed_column_uses_index_and_returns_correct_rows() {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT NOT NULL)").unwrap();
        db.execute("INSERT INTO t (id, name) VALUES (1, 'a'), (2, 'b'), (3, 'a')").unwrap();
        db.execute("CREATE INDEX idx_name ON t (name)").unwrap();

        let result = db.execute("SELECT id FROM t WHERE name = 'a'").unwrap();
        match result {
            ExecResult::Rows { rows, .. } => {
                let mut ids: Vec<i64> = rows.iter().map(|r| match &r[0] { Value::Integer(n) => *n, _ => unreachable!() }).collect();
                ids.sort();
                assert_eq!(ids, vec![1, 3]);
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }
```

- [ ] **Step 8: Run the full test suite**

Run: `cargo test`
Expected: PASS — all tests through Task 35.

- [ ] **Step 9: Commit**

```bash
git add src/exec/scan.rs src/plan/planner.rs src/engine.rs
git commit -m "Add IndexSeek and planner rule to route indexed-column equality WHERE clauses"
```

---

## Task 36: Index equivalence property test

**Files:**
- Create: `tests/index_equivalence.rs`

**Interfaces:**
- Consumes: the public `dbengine::{Database, ExecResult}` API.
- Produces: no new interface — a property test proving that queries return identical results whether or not an index exists on the filtered column. This is the test named explicitly in the design spec to catch planner bugs that single-path correctness tests would miss.

- [ ] **Step 1: Write the property test**

Write `tests/index_equivalence.rs`:

```rust
use dbengine::{Database, ExecResult};
use proptest::prelude::*;
use tempfile::NamedTempFile;

// Returns the Database together with the NamedTempFile that backs it — the
// temp file must stay alive (not be dropped/deleted) for as long as the
// Database's Pager holds an open file handle to it, so the caller must keep
// both bindings in scope for the same duration rather than discarding the
// NamedTempFile immediately.
fn make_db_with_rows(rows: &[(i64, i64)]) -> (Database, NamedTempFile) {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();
    db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, score INTEGER NOT NULL)").unwrap();
    for (id, score) in rows {
        db.execute(&format!("INSERT INTO t (id, score) VALUES ({id}, {score})")).unwrap();
    }
    (db, file)
}

proptest! {
    #[test]
    fn indexed_and_unindexed_queries_agree(
        rows in prop::collection::vec((0i64..500, 0i64..20), 1..100),
        target in 0i64..20,
    ) {
        // de-duplicate ids: the primary key must be unique per row.
        let mut seen = std::collections::HashSet::new();
        let unique_rows: Vec<(i64, i64)> = rows.into_iter().filter(|(id, _)| seen.insert(*id)).collect();

        let query = format!("SELECT id FROM t WHERE score = {target} ORDER BY id");

        let (mut without_index, _file1) = make_db_with_rows(&unique_rows);
        let without_result = without_index.execute(&query).unwrap();

        let (mut with_index, _file2) = make_db_with_rows(&unique_rows);
        with_index.execute("CREATE INDEX idx_score ON t (score)").unwrap();
        let with_result = with_index.execute(&query).unwrap();

        prop_assert_eq!(without_result, with_result);
    }
}
```

Both `_file1` and `_file2` stay bound (not `_`) for the rest of the closure body so they aren't dropped before `execute` runs — the leading underscore only silences the "unused variable" warning, it does not shorten their lifetime.

- [ ] **Step 2: Run the property test**

Run: `cargo test --test index_equivalence`
Expected: PASS. If it fails, the failure is a real planner or `IndexSeek` bug — do not weaken the assertion; fix the underlying operator or planner rule until indexed and unindexed results agree exactly.

- [ ] **Step 3: Commit**

```bash
git add tests/index_equivalence.rs
git commit -m "Add property test proving indexed and unindexed queries return identical results"
```

---

## Task 37: REPL — main loop, statement execution, result formatting

**Files:**
- Modify: `src/repl.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `Database::{create, open, execute}`, `ExecResult`, `Value`.
- Produces: `repl::{format_value, format_result, run}`. `format_value`/`format_result` are pure functions (no I/O), kept separate from `run` specifically so they're unit-testable without driving a real terminal.

- [ ] **Step 1: Write the failing test for the pure formatting functions**

Write `src/repl.rs`:

```rust
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use crate::engine::{Database, ExecResult};
use crate::types::value::Value;

pub fn format_value(v: &Value) -> String {
    match v {
        Value::Integer(i) => i.to_string(),
        Value::Text(s) => s.clone(),
        Value::Boolean(b) => b.to_string(),
        Value::Null => "NULL".to_string(),
    }
}

pub fn format_result(result: &ExecResult) -> String {
    todo_marker();
    unreachable!()
}

fn todo_marker() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_scalar_values() {
        assert_eq!(format_value(&Value::Integer(5)), "5");
        assert_eq!(format_value(&Value::Text("hi".into())), "hi");
        assert_eq!(format_value(&Value::Boolean(true)), "true");
        assert_eq!(format_value(&Value::Null), "NULL");
    }

    #[test]
    fn formats_ok_result() {
        assert_eq!(format_result(&ExecResult::Ok), "OK");
    }

    #[test]
    fn formats_modified_result() {
        assert_eq!(format_result(&ExecResult::Modified(3)), "3 row(s) modified");
    }

    #[test]
    fn formats_rows_result_with_header_and_count() {
        let result = ExecResult::Rows {
            columns: vec!["id".into(), "name".into()],
            rows: vec![
                vec![Value::Integer(1), Value::Text("a".into())],
                vec![Value::Integer(2), Value::Null],
            ],
        };
        let text = format_result(&result);
        assert!(text.contains("id | name"));
        assert!(text.contains("1 | a"));
        assert!(text.contains("2 | NULL"));
        assert!(text.contains("(2 row(s))"));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test repl::tests`
Expected: FAIL — `format_result` is `unreachable!()`.

- [ ] **Step 3: Implement `format_result`**

Replace the placeholder body:

```rust
pub fn format_result(result: &ExecResult) -> String {
    match result {
        ExecResult::Ok => "OK".to_string(),
        ExecResult::Modified(n) => format!("{n} row(s) modified"),
        ExecResult::Rows { columns, rows } => {
            let mut out = String::new();
            out.push_str(&columns.join(" | "));
            out.push('\n');
            for row in rows {
                let cells: Vec<String> = row.iter().map(format_value).collect();
                out.push_str(&cells.join(" | "));
                out.push('\n');
            }
            out.push_str(&format!("({} row(s))", rows.len()));
            out
        }
    }
}
```

Delete the unused `fn todo_marker() {}` helper.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test repl::tests`
Expected: PASS (4 tests)

- [ ] **Step 5: Implement the interactive loop**

Add to `src/repl.rs`, below the formatting functions:

```rust
pub fn run(mut db: Database) {
    let mut rl = DefaultEditor::new().expect("failed to initialize line editor");
    let mut buffer = String::new();

    loop {
        let prompt = if buffer.is_empty() { "dbengine> " } else { "     ...> " };
        match rl.readline(prompt) {
            Ok(line) => {
                let _ = rl.add_history_entry(line.as_str());
                let trimmed = line.trim();

                if buffer.is_empty() && trimmed.starts_with('.') {
                    if trimmed == ".exit" {
                        break;
                    }
                    println!("{}", crate::repl::meta::dispatch(&mut db, &trimmed[1..]));
                    continue;
                }

                buffer.push_str(&line);
                buffer.push('\n');
                if buffer.trim_end().ends_with(';') {
                    match db.execute(buffer.trim()) {
                        Ok(result) => println!("{}", format_result(&result)),
                        Err(e) => println!("error: {e}"),
                    }
                    buffer.clear();
                }
            }
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => break,
            Err(e) => {
                println!("error: {e}");
                break;
            }
        }
    }
}
```

This references `crate::repl::meta::dispatch`, a module that doesn't exist yet — Task 38 adds it. For now, replace that line with a temporary stand-in so the crate compiles:

```rust
                    println!("unknown command: {trimmed}");
                    continue;
```

(Task 38 replaces this line with the real `meta::dispatch` call.)

- [ ] **Step 6: Wire the REPL into `main.rs`**

Replace `src/main.rs`:

```rust
use std::env;
use std::process::ExitCode;

use dbengine::Database;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let Some(path) = args.get(1) else {
        eprintln!("usage: dbengine <database-file>");
        return ExitCode::FAILURE;
    };

    let path = std::path::Path::new(path);
    let db = if path.exists() {
        Database::open(path)
    } else {
        Database::create(path)
    };

    match db {
        Ok(db) => {
            dbengine::repl::run(db);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error opening database: {e}");
            ExitCode::FAILURE
        }
    }
}
```

Add `pub mod repl;` to `src/lib.rs` (it was declared as a bare file with no `mod` statement referencing it in Task 1 — `src/lib.rs` must list it as a module for `main.rs` to reach `dbengine::repl::run`).

- [ ] **Step 7: Verify the project builds and the REPL runs manually**

Run: `cargo build`
Expected: compiles with no errors.

Run: `cargo run -- /tmp/manual_test.db` (or an equivalent temp path), then at the `dbengine>` prompt type `CREATE TABLE t (id INTEGER PRIMARY KEY);` followed by `.exit`.
Expected: prints `OK`, then exits cleanly with no panic.

- [ ] **Step 8: Commit**

```bash
git add src/repl.rs src/main.rs src/lib.rs
git commit -m "Add REPL main loop with statement buffering and result formatting"
```

---

## Task 38: Meta-commands — .tables, .schema, .indexes, .exit

**Files:**
- Create: `src/repl/meta.rs`
- Modify: `src/repl.rs`
- Modify: `src/engine.rs`

**Interfaces:**
- Consumes: `Database`, `TableSchema`, `IndexSchema`.
- Produces: `Database::{list_tables, table_schema, list_indexes}` accessor methods. `meta::dispatch(&mut Database, &str) -> String` handling `tables`, `schema [table]`, `indexes [table]`, and an `unknown command` fallback (`.exit` stays handled directly in `repl::run`, since it needs to break the loop rather than print text).

- [ ] **Step 1: Add engine accessor methods**

Add to `impl Database` in `src/engine.rs`:

```rust
    pub fn list_tables(&mut self) -> Vec<String> {
        self.catalog.list_tables(&mut self.pager).unwrap_or_default()
    }

    pub fn table_schema(&mut self, name: &str) -> Option<TableSchema> {
        self.catalog.get_table(&mut self.pager, name).ok().flatten()
    }

    pub fn list_indexes(&mut self, table: &str) -> Vec<crate::types::schema::IndexSchema> {
        self.catalog.list_indexes_for_table(&mut self.pager, table).unwrap_or_default()
    }
```

- [ ] **Step 2: Write the failing test for `meta::dispatch`**

Write `src/repl/meta.rs`:

```rust
use crate::engine::Database;

pub fn dispatch(db: &mut Database, cmd: &str) -> String {
    todo_marker();
    unreachable!()
}

fn todo_marker() {}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn db_with_one_table() -> Database {
        let file = NamedTempFile::new().unwrap();
        let mut db = Database::create(file.path()).unwrap();
        std::mem::forget(file); // acceptable here: test runs and drops db within the same call, no cross-scope reopen
        db.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)").unwrap();
        db
    }

    #[test]
    fn tables_lists_table_names() {
        let mut db = db_with_one_table();
        assert_eq!(dispatch(&mut db, "tables"), "users");
    }

    #[test]
    fn schema_shows_create_table_text() {
        let mut db = db_with_one_table();
        let out = dispatch(&mut db, "schema users");
        assert!(out.contains("CREATE TABLE users"));
        assert!(out.contains("id INTEGER"));
        assert!(out.contains("PRIMARY KEY"));
        assert!(out.contains("name TEXT"));
        assert!(out.contains("NOT NULL"));
    }

    #[test]
    fn schema_missing_table_reports_error() {
        let mut db = db_with_one_table();
        assert!(dispatch(&mut db, "schema nope").contains("no such table"));
    }

    #[test]
    fn indexes_lists_indexes_for_table() {
        let mut db = db_with_one_table();
        db.execute("CREATE INDEX idx_name ON users (name)").unwrap();
        let out = dispatch(&mut db, "indexes users");
        assert!(out.contains("idx_name"));
    }

    #[test]
    fn unknown_command_reports_error() {
        let mut db = db_with_one_table();
        assert!(dispatch(&mut db, "bogus").contains("unknown command"));
    }
}
```

Note the `std::mem::forget(file)` in `db_with_one_table`: unlike Task 36's cross-database comparison (which genuinely needed two independent temp files to outlive their creation scope), every test here creates and fully consumes `db` within a single test function body, so leaking the temp file is a deliberate, scoped simplification — the OS reclaims temp directory space on the next reboot regardless, and no test here reopens the file from a second `Database` handle the way Task 41's persistence test will. Task 41 must not reuse this pattern.

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test repl::meta::tests`
Expected: FAIL — `dispatch` is `unreachable!()`; also a compile error since `src/repl.rs` doesn't yet declare `pub mod meta;`.

Add `pub mod meta;` to the top of `src/repl.rs` before running the test.

- [ ] **Step 4: Implement `dispatch`**

Replace the placeholder body in `src/repl/meta.rs`:

```rust
pub fn dispatch(db: &mut Database, cmd: &str) -> String {
    let cmd = cmd.trim();
    if cmd == "tables" {
        return db.list_tables().join("\n");
    }
    if let Some(rest) = cmd.strip_prefix("schema") {
        return schema_text(db, rest.trim());
    }
    if let Some(rest) = cmd.strip_prefix("indexes") {
        return indexes_text(db, rest.trim());
    }
    format!("unknown command: .{cmd}")
}

fn schema_text(db: &mut Database, table: &str) -> String {
    match db.table_schema(table) {
        Some(schema) => {
            let cols: Vec<String> = schema
                .columns
                .iter()
                .map(|c| {
                    let ty = match c.ty {
                        crate::types::value::ColumnType::Integer => "INTEGER",
                        crate::types::value::ColumnType::Text => "TEXT",
                        crate::types::value::ColumnType::Boolean => "BOOLEAN",
                    };
                    let mut parts = vec![c.name.clone(), ty.to_string()];
                    if c.not_null {
                        parts.push("NOT NULL".to_string());
                    }
                    if c.is_primary_key {
                        parts.push("PRIMARY KEY".to_string());
                    }
                    parts.join(" ")
                })
                .collect();
            format!("CREATE TABLE {} ({})", schema.name, cols.join(", "))
        }
        None => format!("no such table: {table}"),
    }
}

fn indexes_text(db: &mut Database, table: &str) -> String {
    let indexes = db.list_indexes(table);
    if indexes.is_empty() {
        return "(no indexes)".to_string();
    }
    indexes
        .iter()
        .map(|i| format!("{} ON {} ({})", i.name, i.table, i.column))
        .collect::<Vec<_>>()
        .join("\n")
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test repl::meta::tests`
Expected: PASS (5 tests)

- [ ] **Step 6: Wire `meta::dispatch` into the REPL loop**

In `src/repl.rs`'s `run` function, replace the Task 37 stand-in:

```rust
                    println!("unknown command: {trimmed}");
                    continue;
```

with:

```rust
                    println!("{}", meta::dispatch(&mut db, &trimmed[1..]));
                    continue;
```

- [ ] **Step 7: Run the full test suite**

Run: `cargo test`
Expected: PASS — all tests through Task 38.

- [ ] **Step 8: Commit**

```bash
git add src/repl/meta.rs src/repl.rs src/engine.rs
git commit -m "Add .tables, .schema, and .indexes REPL meta-commands"
```

---

## Task 39: Meta-command — .btree dump

**Files:**
- Modify: `src/btree/tree.rs`
- Modify: `src/engine.rs`
- Modify: `src/repl/meta.rs`

**Interfaces:**
- Consumes: `BTree`, `Database::table_schema`.
- Produces: `BTree::dump(&mut self) -> String` — a human-readable tree walk showing page numbers, node kind, height, and key count per node. `Database::dump_table_btree(&mut self, table: &str) -> String`. `meta::dispatch` gains a `btree <table>` command.

- [ ] **Step 1: Write the failing test for `BTree::dump`**

Add to the `tests` module in `src/btree/tree.rs`:

```rust
    #[test]
    fn dump_shows_every_node_and_reports_split_height() {
        let (mut pager, root) = empty_tree();
        let mut bt = BTree::new(&mut pager, root);
        for i in 0..400i64 {
            bt.insert(&(i as u64).to_be_bytes(), b"v").unwrap();
        }
        let dump = bt.dump();
        assert!(dump.contains("leaf"));
        assert!(dump.contains("internal"), "400 small keys must have split into a multi-level tree");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test btree::tree::tests::dump_shows_every_node_and_reports_split_height`
Expected: FAIL — `dump` not defined.

- [ ] **Step 3: Implement `dump`**

Add to `impl<'a> BTree<'a>`:

```rust
    pub fn dump(&mut self) -> String {
        let mut out = String::new();
        self.dump_node(self.root, 0, &mut out);
        out
    }

    fn dump_node(&mut self, page_no: u32, depth: usize, out: &mut String) {
        let page = match self.pager.get_page(page_no) {
            Ok(p) => p.clone(),
            Err(e) => {
                out.push_str(&format!("{}page {page_no}: error reading page: {e}\n", "  ".repeat(depth)));
                return;
            }
        };
        let indent = "  ".repeat(depth);
        match page.page_type() {
            crate::storage::page::PAGE_TYPE_LEAF => {
                let node = LeafNode::decode(&page);
                out.push_str(&format!("{indent}leaf page {page_no}: {} entries\n", node.entries.len()));
            }
            crate::storage::page::PAGE_TYPE_INTERNAL => {
                let node = InternalNode::decode(&page);
                out.push_str(&format!(
                    "{indent}internal page {page_no}: {} keys, {} children\n",
                    node.entries.len(),
                    node.entries.len() + 1
                ));
                for e in &node.entries {
                    self.dump_node(e.left_child, depth + 1, out);
                }
                self.dump_node(node.rightmost_child, depth + 1, out);
            }
            t => out.push_str(&format!("{indent}page {page_no}: unknown page type {t}\n")),
        }
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test btree::tree::tests::dump_shows_every_node_and_reports_split_height`
Expected: PASS

- [ ] **Step 5: Add `Database::dump_table_btree` and wire the REPL command**

Add to `impl Database` in `src/engine.rs`:

```rust
    pub fn dump_table_btree(&mut self, table: &str) -> Option<String> {
        let schema = self.table_schema(table)?;
        let mut bt = crate::btree::tree::BTree::new(&mut self.pager, schema.root_page);
        Some(bt.dump())
    }
```

In `src/repl/meta.rs`, add to `dispatch`:

```rust
    if let Some(rest) = cmd.strip_prefix("btree") {
        return btree_text(db, rest.trim());
    }
```

And add:

```rust
fn btree_text(db: &mut Database, table: &str) -> String {
    match db.dump_table_btree(table) {
        Some(text) => text,
        None => format!("no such table: {table}"),
    }
}
```

- [ ] **Step 6: Write and run a REPL-level `.btree` test**

Add to `src/repl/meta.rs`'s `tests` module:

```rust
    #[test]
    fn btree_dumps_table_structure() {
        let mut db = db_with_one_table();
        for i in 0..50 {
            db.execute(&format!("INSERT INTO users (id, name) VALUES ({i}, 'n{i}')")).unwrap();
        }
        let out = dispatch(&mut db, "btree users");
        assert!(out.contains("leaf page"));
    }
```

Run: `cargo test repl::`
Expected: PASS (6 tests)

- [ ] **Step 7: Commit**

```bash
git add src/btree/tree.rs src/engine.rs src/repl/meta.rs
git commit -m "Add .btree meta-command showing tree structure and height"
```

---

## Task 40: Meta-command — .stats

**Files:**
- Modify: `src/engine.rs`
- Modify: `src/repl/meta.rs`

**Interfaces:**
- Consumes: `Pager::{stats, reset_read_counter}`, `PagerStats`.
- Produces: `Database::{pager_stats, reset_read_counter}`. `meta::dispatch` gains a `stats` command.

- [ ] **Step 1: Add engine accessor methods**

Add to `impl Database` in `src/engine.rs`:

```rust
    pub fn pager_stats(&self) -> crate::storage::pager::PagerStats {
        self.pager.stats()
    }

    pub fn reset_read_counter(&mut self) {
        self.pager.reset_read_counter();
    }
```

- [ ] **Step 2: Write the failing test**

Add to the `tests` module in `src/repl/meta.rs`:

```rust
    #[test]
    fn stats_reports_page_count_and_reads() {
        let mut db = db_with_one_table();
        let out = dispatch(&mut db, "stats");
        assert!(out.contains("pages:"));
        assert!(out.contains("freelist:"));
        assert!(out.contains("pages read since last statement:"));
    }
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test repl::meta::tests::stats_reports_page_count_and_reads`
Expected: FAIL — `.stats` falls into the `unknown command` branch.

- [ ] **Step 4: Implement**

Add to `dispatch` in `src/repl/meta.rs`:

```rust
    if cmd == "stats" {
        return stats_text(db);
    }
```

And add:

```rust
fn stats_text(db: &mut Database) -> String {
    let s = db.pager_stats();
    format!(
        "pages: {}\nfreelist: {}\ncached pages: {}\npages read since last statement: {}",
        s.page_count, s.freelist_head, s.cached_pages, s.pages_read
    )
}
```

- [ ] **Step 5: Reset the read counter at the start of every statement**

For `.stats`'s "pages read since last statement" to be meaningful (per the design spec's success criteria — demonstrating that an indexed query touches fewer pages), the counter must reset before each `db.execute(...)` call in the REPL loop, not just at startup.

In `src/repl.rs`'s `run` function, change the statement-execution branch:

```rust
                if buffer.trim_end().ends_with(';') {
                    db.reset_read_counter();
                    match db.execute(buffer.trim()) {
                        Ok(result) => println!("{}", format_result(&result)),
                        Err(e) => println!("error: {e}"),
                    }
                    buffer.clear();
                }
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test repl::`
Expected: PASS (7 tests)

- [ ] **Step 7: Manually verify the index-vs-scan page count difference**

Run: `cargo build`, then `cargo run -- <temp-path>.db`, and at the prompt:

```
CREATE TABLE t (id INTEGER PRIMARY KEY, val INTEGER NOT NULL);
```

then insert a few thousand rows via a shell loop or repeated `INSERT` statements, then compare `.stats` output for `SELECT id FROM t WHERE val = 500;` before and after `CREATE INDEX idx_val ON t (val);`.
Expected: pages-read is visibly lower after the index exists — this is the concrete demonstration named in the design spec's success criteria.

- [ ] **Step 8: Commit**

```bash
git add src/engine.rs src/repl/meta.rs src/repl.rs
git commit -m "Add .stats meta-command and per-statement page-read tracking"
```

---

## Task 41: Persistence and durability tests

**Files:**
- Create: `tests/persistence.rs`

**Interfaces:**
- Consumes: the public `dbengine::{Database, ExecResult}` API.
- Produces: no new interface — proof that data survives a full process-level close/reopen cycle, which is the test the design spec calls out as proving "it is a database and not a data structure."

- [ ] **Step 1: Write the failing test**

Write `tests/persistence.rs`:

```rust
use dbengine::types::value::Value;
use dbengine::{Database, ExecResult};
use tempfile::NamedTempFile;

#[test]
fn data_survives_full_close_and_reopen() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path().to_path_buf();

    {
        let mut db = Database::create(&path).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT NOT NULL)").unwrap();
        db.execute("INSERT INTO t (id, name) VALUES (1, 'a'), (2, 'b'), (3, 'c')").unwrap();
        db.execute("CREATE INDEX idx_name ON t (name)").unwrap();
        // db is dropped here, closing its Pager's file handle — no explicit close() exists
        // or is needed, since every statement already flushes and fsyncs on commit.
    }

    let mut db = Database::open(&path).unwrap();
    let result = db.execute("SELECT id, name FROM t WHERE id = 2").unwrap();
    assert_eq!(
        result,
        ExecResult::Rows { columns: vec!["id".into(), "name".into()], rows: vec![vec![Value::Integer(2), Value::Text("b".into())]] }
    );

    // the index built before close must still route through IndexSeek after reopen —
    // this is the same query Task 35's engine test uses, now run against a reopened file.
    let result = db.execute("SELECT id FROM t WHERE name = 'c'").unwrap();
    assert_eq!(result, ExecResult::Rows { columns: vec!["id".into()], rows: vec![vec![Value::Integer(3)]] });

    db.execute("DELETE FROM t WHERE id = 1").unwrap();
    drop(db);

    let mut db = Database::open(&path).unwrap();
    let result = db.execute("SELECT id FROM t").unwrap();
    match result {
        ExecResult::Rows { rows, .. } => assert_eq!(rows.len(), 2, "delete before close must also have persisted"),
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn every_statement_is_durable_even_without_explicit_flush_call() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path().to_path_buf();

    let mut db = Database::create(&path).unwrap();
    db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)").unwrap();
    for i in 0..50 {
        db.execute(&format!("INSERT INTO t (id) VALUES ({i})")).unwrap();
        // Reopening after every single statement (not just at the end) proves each
        // individual execute() call is fsynced on its own, per the autocommit design.
        drop(Database::open(&path).unwrap());
    }
    drop(db);

    let mut db = Database::open(&path).unwrap();
    let result = db.execute("SELECT id FROM t").unwrap();
    match result {
        ExecResult::Rows { rows, .. } => assert_eq!(rows.len(), 50),
        other => panic!("unexpected result: {other:?}"),
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test persistence`
Expected: FAIL if any earlier task's flush/fsync wiring has a gap — otherwise this may already PASS, since `Database::execute` has called `self.pager.flush()` after every statement since Task 26. Either outcome is informative: if it fails, something upstream regressed; if it passes immediately, this task still adds durable regression coverage for that guarantee.

- [ ] **Step 3: Fix any failure, or confirm the pass**

If the test fails, the bug is almost certainly a missing `self.pager.flush()?` call in one of `execute_create_table`, `execute_insert`, `execute_select`, `execute_update`, `execute_delete`, `execute_create_index`, or `execute_drop_index` — but per Task 26's `execute` method, `flush()` is called once centrally after the `match` in every code path, so no per-statement method should need its own flush call. Re-check `src/engine.rs`'s `execute` function structure if this test fails.

Run: `cargo test --test persistence`
Expected: PASS (2 tests)

- [ ] **Step 4: Commit**

```bash
git add tests/persistence.rs
git commit -m "Add persistence tests proving data survives close/reopen cycles"
```

---

## Task 42: README documenting scope and limitations

**Files:**
- Create: `README.md`

**Interfaces:**
- Consumes: nothing — this is documentation, not code.
- Produces: a top-level `README.md` stating, plainly, every limitation named in the Global Constraints section of this plan and the design spec.

- [ ] **Step 1: Write `README.md`**

```markdown
# dbengine

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
  The natural next step — recorded but not built — is a rollback journal:
  copy each page's original bytes to a side file and `fsync` it before
  modifying the page in place, then delete the journal on commit or replay it
  on reopen after a crash.
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
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "Add README documenting usage, architecture, and scope limitations"
```

---

## Task 43: Scale test against the design spec's success criteria

**Files:**
- Create: `tests/scale.rs`

**Interfaces:**
- Consumes: the public `dbengine::{Database, ExecResult}` API.
- Produces: no new interface — an automated check of every success criterion listed in the design spec: 100,000 rows created/populated/queried/updated/deleted via SQL, the file surviving reopen, `.btree`-equivalent height verification, and an index measurably reducing pages read.

This test is slow (100,000 individual `INSERT` statements, each parsed, planned, and fsynced). Mark it `#[ignore]` so `cargo test` stays fast by default; run it explicitly with `cargo test -- --ignored`.

- [ ] **Step 1: Write the test**

Write `tests/scale.rs`:

```rust
use dbengine::types::value::Value;
use dbengine::{Database, ExecResult};
use tempfile::NamedTempFile;

#[test]
#[ignore]
fn hundred_thousand_rows_create_populate_query_update_delete_reopen() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path().to_path_buf();

    {
        let mut db = Database::create(&path).unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val INTEGER NOT NULL)").unwrap();
        for i in 0..100_000i64 {
            db.execute(&format!("INSERT INTO t (id, val) VALUES ({i}, {})", i % 1000)).unwrap();
        }

        let result = db.execute("SELECT id FROM t WHERE id = 54321").unwrap();
        assert_eq!(result, ExecResult::Rows { columns: vec!["id".into()], rows: vec![vec![Value::Integer(54321)]] });

        assert_eq!(db.execute("UPDATE t SET val = 9999 WHERE id = 1").unwrap(), ExecResult::Modified(1));
        assert_eq!(db.execute("DELETE FROM t WHERE id = 2").unwrap(), ExecResult::Modified(1));

        // Tree height check, equivalent to what `.btree` shows interactively: dump
        // the table's B+Tree and confirm it went past a single level.
        let dump = db.dump_table_btree("t").unwrap();
        assert!(dump.lines().any(|l| l.trim_start().starts_with("internal")), "100k rows must produce a multi-level tree");
    }

    // Reopen from disk and verify the data, including the update and delete, survived.
    let mut db = Database::open(&path).unwrap();
    let result = db.execute("SELECT val FROM t WHERE id = 1").unwrap();
    assert_eq!(result, ExecResult::Rows { columns: vec!["val".into()], rows: vec![vec![Value::Integer(9999)]] });
    let result = db.execute("SELECT id FROM t WHERE id = 2").unwrap();
    assert_eq!(result, ExecResult::Rows { columns: vec!["id".into()], rows: vec![] });

    // Index page-read reduction: compare pages read for the same query before and
    // after CREATE INDEX, directly demonstrating the design spec's success criterion.
    db.reset_read_counter();
    db.execute("SELECT id FROM t WHERE val = 500").unwrap();
    let pages_without_index = db.pager_stats().pages_read;

    db.execute("CREATE INDEX idx_val ON t (val)").unwrap();

    db.reset_read_counter();
    db.execute("SELECT id FROM t WHERE val = 500").unwrap();
    let pages_with_index = db.pager_stats().pages_read;

    assert!(
        pages_with_index < pages_without_index,
        "indexed lookup ({pages_with_index} pages) should read fewer pages than the sequential scan ({pages_without_index} pages)"
    );
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test --test scale -- --ignored --nocapture`
Expected: PASS. This may take from several seconds to a couple of minutes depending on hardware, since it performs 100,000 individually-parsed, individually-fsynced `INSERT` statements — that per-statement fsync cost is itself a direct, honest consequence of the autocommit durability model documented in the README, not a test artifact to work around.

- [ ] **Step 3: Run the entire test suite one final time**

Run: `cargo test`
Expected: PASS — every task's tests, Task 1 through Task 42, green together. This is the final verification that the whole engine — pager, B+Tree, catalog, parser, planner, executor, and REPL — works as one coherent system.

- [ ] **Step 4: Commit**

```bash
git add tests/scale.rs
git commit -m "Add 100k-row scale test verifying every design spec success criterion"
```
