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
            pager.set_catalog_root(9).unwrap();
        }
        let pager = Pager::open(path).unwrap();
        assert_eq!(pager.catalog_root(), 9);
    }

    #[test]
    fn get_page_mut_then_get_page_sees_write() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let no = pager.allocate_page().unwrap();
        {
            let page = pager.get_page_mut(no).unwrap();
            page.write_u8(0, 42);
        }
        let page = pager.get_page(no).unwrap();
        assert_eq!(page.read_u8(0), 42);
    }

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
}
