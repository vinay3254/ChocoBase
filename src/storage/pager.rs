use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::error::StorageError;
use crate::storage::header::Header;
use crate::storage::journal::{self, Journal};
use crate::storage::page::{Page, PAGE_SIZE};

const DEFAULT_CACHE_CAPACITY: usize = 256;

pub struct Pager {
    file: File,
    path: PathBuf,
    cache: HashMap<u32, Page>,
    recency: VecDeque<u32>,
    dirty: HashSet<u32>,
    header: Header,
    capacity: usize,
    pub pages_read: u64,

    in_transaction: bool,
    journal: Option<Journal>,
    journaled_pages: HashSet<u32>,
    orig_page_count: u32,
}

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
            path: path.to_path_buf(),
            cache: HashMap::new(),
            recency: VecDeque::new(),
            dirty: HashSet::new(),
            header: Header::new(),
            capacity: DEFAULT_CACHE_CAPACITY,
            pages_read: 0,
            in_transaction: false,
            journal: None,
            journaled_pages: HashSet::new(),
            orig_page_count: 1,
        };
        pager.flush_header()?;
        pager.file.sync_all()?;
        Ok(pager)
    }

    pub fn open(path: &Path) -> Result<Self, StorageError> {
        journal::recover_if_needed(path)?;

        let mut file = OpenOptions::new().read(true).write(true).open(path)?;
        let mut buf = [0u8; PAGE_SIZE];
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut buf)?;
        let page = Page { data: buf };
        let header = Header::read_from(&page)?;
        Ok(Pager {
            file,
            path: path.to_path_buf(),
            cache: HashMap::new(),
            recency: VecDeque::new(),
            dirty: HashSet::new(),
            header,
            capacity: DEFAULT_CACHE_CAPACITY,
            pages_read: 0,
            in_transaction: false,
            journal: None,
            journaled_pages: HashSet::new(),
            orig_page_count: 0,
        })
    }

    pub fn in_transaction(&self) -> bool {
        self.in_transaction
    }

    pub fn begin_transaction(&mut self) -> Result<(), StorageError> {
        if self.in_transaction {
            return Err(StorageError::CorruptJournal(
                "transaction already in progress".into(),
            ));
        }
        self.orig_page_count = self.header.page_count;
        self.journal = Some(Journal::create(&self.path, self.orig_page_count)?);
        self.journaled_pages.clear();
        self.in_transaction = true;
        Ok(())
    }

    pub fn commit_transaction(&mut self) -> Result<(), StorageError> {
        if !self.in_transaction {
            return Ok(());
        }
        if let Some(mut jnl) = self.journal.take() {
            jnl.sync()?;
            self.flush()?;
            jnl.close_and_delete()?;
        }
        self.journaled_pages.clear();
        self.in_transaction = false;
        Ok(())
    }

    pub fn rollback_transaction(&mut self) -> Result<(), StorageError> {
        if !self.in_transaction {
            return Ok(());
        }
        if let Some(jnl) = self.journal.take() {
            drop(jnl);
        }
        journal::recover_if_needed(&self.path)?;

        self.cache.clear();
        self.recency.clear();
        self.dirty.clear();

        let page0 = self.read_page_from_disk(0)?;
        self.header = Header::read_from(&page0)?;
        self.journaled_pages.clear();
        self.in_transaction = false;
        Ok(())
    }

    pub fn catalog_root(&self) -> u32 {
        self.header.catalog_root
    }

    pub fn set_catalog_root(&mut self, root: u32) -> Result<(), StorageError> {
        self.record_preimage_for_header()?;
        self.header.catalog_root = root;
        self.flush_header()
    }

    fn record_preimage_for_header(&mut self) -> Result<(), StorageError> {
        if self.in_transaction && !self.journaled_pages.contains(&0) {
            let mut page0 = Page::zeroed();
            self.header.write_to(&mut page0);
            if let Some(jnl) = &mut self.journal {
                jnl.append_page(0, &page0)?;
            }
            self.journaled_pages.insert(0);
        }
        Ok(())
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
        self.file
            .seek(SeekFrom::Start(no as u64 * PAGE_SIZE as u64))?;
        self.file.read_exact(&mut buf)?;
        Ok(Page { data: buf })
    }

    fn write_page_to_disk(&mut self, no: u32) -> Result<(), StorageError> {
        let page = self.cache.get(&no).expect("dirty page must be cached");
        self.file
            .seek(SeekFrom::Start(no as u64 * PAGE_SIZE as u64))?;
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
        if self.in_transaction && !self.journaled_pages.contains(&no) {
            if no < self.orig_page_count {
                let page_copy = self.cache.get(&no).unwrap().clone();
                if let Some(jnl) = &mut self.journal {
                    jnl.append_page(no, &page_copy)?;
                }
            }
            self.journaled_pages.insert(no);
        }
        self.dirty.insert(no);
        Ok(self.cache.get_mut(&no).unwrap())
    }

    pub fn allocate_page(&mut self) -> Result<u32, StorageError> {
        self.record_preimage_for_header()?;
        let no = if self.header.freelist_head != 0 {
            let free_no = self.header.freelist_head;
            self.ensure_loaded(free_no)?;
            if self.in_transaction
                && !self.journaled_pages.contains(&free_no)
                && free_no < self.orig_page_count
            {
                let page_copy = self.cache.get(&free_no).unwrap().clone();
                if let Some(jnl) = &mut self.journal {
                    jnl.append_page(free_no, &page_copy)?;
                }
                self.journaled_pages.insert(free_no);
            }
            let next = self.cache.get(&free_no).unwrap().read_u32(0);
            self.header.freelist_head = next;
            self.flush_header()?;
            free_no
        } else {
            let no = self.header.page_count;
            self.header.page_count += 1;
            self.flush_header()?;
            self.file
                .set_len(self.header.page_count as u64 * PAGE_SIZE as u64)?;
            no
        };
        self.evict_if_needed()?;
        self.cache.insert(no, Page::zeroed());
        self.dirty.insert(no);
        self.touch(no);
        Ok(no)
    }

    pub fn free_page(&mut self, no: u32) -> Result<(), StorageError> {
        self.record_preimage_for_header()?;
        self.ensure_loaded(no)?;
        if self.in_transaction && !self.journaled_pages.contains(&no) && no < self.orig_page_count {
            let page_copy = self.cache.get(&no).unwrap().clone();
            if let Some(jnl) = &mut self.journal {
                jnl.append_page(no, &page_copy)?;
            }
            self.journaled_pages.insert(no);
        }

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
}

#[derive(Debug, Clone, Copy)]
pub struct PagerStats {
    pub page_count: u32,
    pub freelist_head: u32,
    pub cached_pages: usize,
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
            assert_eq!(pager.catalog_root(), 0);
            pager.set_catalog_root(42).unwrap();
        }
        {
            let pager = Pager::open(path).unwrap();
            assert_eq!(pager.catalog_root(), 42);
        }
    }

    #[test]
    fn allocate_and_free_pages_reuses_freelist() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path();
        let mut pager = Pager::create(path).unwrap();

        let p1 = pager.allocate_page().unwrap();
        let p2 = pager.allocate_page().unwrap();
        assert_eq!(p1, 1);
        assert_eq!(p2, 2);
        assert_eq!(pager.header.page_count, 3);

        pager.free_page(p2).unwrap();
        assert_eq!(pager.header.freelist_head, 2);

        let p3 = pager.allocate_page().unwrap();
        assert_eq!(p3, 2, "freed page 2 should be reused");
        assert_eq!(pager.header.freelist_head, 0);
        assert_eq!(
            pager.header.page_count, 3,
            "page count shouldn't have grown"
        );
    }

    #[test]
    fn transaction_rollback_restores_preimages() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path();
        let mut pager = Pager::create(path).unwrap();

        let p1 = pager.allocate_page().unwrap();
        {
            let page = pager.get_page_mut(p1).unwrap();
            page.write_u32(0, 100);
        }
        pager.flush().unwrap();

        // Start transaction and modify
        pager.begin_transaction().unwrap();
        {
            let page = pager.get_page_mut(p1).unwrap();
            page.write_u32(0, 999);
        }
        let p2 = pager.allocate_page().unwrap();
        {
            let page = pager.get_page_mut(p2).unwrap();
            page.write_u32(0, 777);
        }

        // Rollback
        pager.rollback_transaction().unwrap();

        // Verify state is restored
        assert_eq!(pager.get_page(p1).unwrap().read_u32(0), 100);
        assert_eq!(pager.header.page_count, 2);
    }

    #[test]
    fn injected_write_failure_during_transaction_mutation_surfaces_clean_error_and_does_not_leak_to_db_file(
    ) {
        use crate::storage::journal::{journal_path_for, JournalWriter};
        use std::io::{Seek, SeekFrom, Write};

        struct FailingWriter {
            inner: File,
            allow_bytes: usize,
            written_bytes: usize,
        }

        impl Write for FailingWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                if self.written_bytes + buf.len() > self.allow_bytes {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::StorageFull,
                        "injected disk full error during journal write",
                    ));
                }
                let n = self.inner.write(buf)?;
                self.written_bytes += n;
                Ok(n)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                self.inner.flush()
            }
        }

        impl Seek for FailingWriter {
            fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
                self.inner.seek(pos)
            }
        }

        impl JournalWriter for FailingWriter {
            fn sync_all(&mut self) -> std::io::Result<()> {
                self.inner.sync_all()
            }
        }

        let file = NamedTempFile::new().unwrap();
        let path = file.path();
        let mut pager = Pager::create(path).unwrap();

        let p1 = pager.allocate_page().unwrap();
        {
            let page = pager.get_page_mut(p1).unwrap();
            page.write_u32(0, 42);
        }
        pager.flush().unwrap();

        // 1. Begin transaction
        pager.begin_transaction().unwrap();

        // 2. Inject a failing writer that succeeds writing the 64-byte header but fails on record append
        let jnl_path = journal_path_for(path);
        let jnl_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&jnl_path)
            .unwrap();
        let failing_writer = FailingWriter {
            inner: jnl_file,
            allow_bytes: crate::storage::journal::JOURNAL_HEADER_SIZE,
            written_bytes: 0,
        };
        let injected_journal =
            Journal::new_with_writer(Box::new(failing_writer), jnl_path, pager.header.page_count)
                .unwrap();
        pager.journal = Some(injected_journal);

        // 3. Attempt to mutate page 1: must cleanly return Err(StorageError::Io) with StorageFull
        let res = pager.get_page_mut(p1);
        match res {
            Err(StorageError::Io(e)) => assert_eq!(e.kind(), std::io::ErrorKind::StorageFull),
            other => panic!(
                "expected StorageError::Io(StorageFull), got {:?}",
                other.is_err()
            ),
        }

        // 4. Verify dirty set does NOT contain p1 and page was not mutated in live db file
        assert!(!pager.dirty.contains(&p1));

        drop(pager);

        // 5. Open database fresh: recovery runs, pre-transaction state is intact (p1 = 42)
        let mut reopened = Pager::open(path).unwrap();
        assert_eq!(reopened.get_page(p1).unwrap().read_u32(0), 42);
    }
}
