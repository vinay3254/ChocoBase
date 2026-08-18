//! Bounded, thread-safe buffer pool manager for ChocoBase.
//!
//! Provides frame latching (`RwLock` over page data), atomic pin tracking,
//! and unpinned frame eviction (LRU / clock).

use std::collections::{HashMap, VecDeque};
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};

use crate::error::StorageError;
use crate::storage::page::Page;

struct Frame {
    page: RwLock<Page>,
    pin_count: AtomicUsize,
    dirty: AtomicBool,
    latch: Mutex<LatchState>,
    changed: Condvar,
}

#[derive(Default)]
struct LatchState {
    readers: usize,
    writer: bool,
}

impl Frame {
    fn new(page: Page) -> Self {
        Self {
            page: RwLock::new(page),
            pin_count: AtomicUsize::new(0),
            dirty: AtomicBool::new(false),
            latch: Mutex::new(LatchState::default()),
            changed: Condvar::new(),
        }
    }
}

/// A bounded, thread-safe page frame cache. The pager remains responsible for
/// disk I/O; this type supplies frame latching, pin tracking, and eviction.
pub struct BufferPool {
    capacity: usize,
    frames: RwLock<HashMap<u32, Arc<Frame>>>,
    recency: Mutex<VecDeque<u32>>,
}

impl BufferPool {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "buffer pool capacity must be greater than 0");
        Self {
            capacity,
            frames: RwLock::new(HashMap::new()),
            recency: Mutex::new(VecDeque::new()),
        }
    }

    pub fn insert(&self, page_no: u32, page: Page) -> Result<(), StorageError> {
        {
            let frames = self.frames.read().unwrap();
            if frames.contains_key(&page_no) {
                return Ok(());
            }
        }
        self.evict_one()?;
        self.frames
            .write()
            .unwrap()
            .insert(page_no, Arc::new(Frame::new(page)));
        self.touch(page_no);
        Ok(())
    }

    pub fn fetch(&self, page_no: u32) -> Option<PageReadGuard> {
        let frame = self.frames.read().unwrap().get(&page_no).cloned()?;
        frame.pin_count.fetch_add(1, Ordering::AcqRel);
        self.touch(page_no);
        let mut latch = frame.latch.lock().unwrap();
        while latch.writer {
            latch = frame.changed.wait(latch).unwrap();
        }
        latch.readers += 1;
        drop(latch);
        let page = frame.page.read().unwrap().clone();
        Some(PageReadGuard { frame, page })
    }

    pub fn fetch_mut(&self, page_no: u32) -> Option<PageWriteGuard> {
        let frame = self.frames.read().unwrap().get(&page_no).cloned()?;
        frame.pin_count.fetch_add(1, Ordering::AcqRel);
        self.touch(page_no);
        let mut latch = frame.latch.lock().unwrap();
        while latch.writer || latch.readers > 0 {
            latch = frame.changed.wait(latch).unwrap();
        }
        latch.writer = true;
        drop(latch);
        let page = frame.page.read().unwrap().clone();
        Some(PageWriteGuard { frame, page })
    }

    pub fn mark_dirty(&self, page_no: u32) {
        if let Some(frame) = self.frames.read().unwrap().get(&page_no) {
            frame.dirty.store(true, Ordering::Release);
        }
    }

    pub fn is_dirty(&self, page_no: u32) -> bool {
        self.frames
            .read()
            .unwrap()
            .get(&page_no)
            .map(|f| f.dirty.load(Ordering::Acquire))
            .unwrap_or(false)
    }

    pub fn contains(&self, page_no: u32) -> bool {
        self.frames.read().unwrap().contains_key(&page_no)
    }

    pub fn len(&self) -> usize {
        self.frames.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn touch(&self, page_no: u32) {
        let mut recency = self.recency.lock().unwrap();
        recency.retain(|p| *p != page_no);
        recency.push_back(page_no);
    }

    fn evict_one(&self) -> Result<(), StorageError> {
        if self.frames.read().unwrap().len() < self.capacity {
            return Ok(());
        }
        let mut recency = self.recency.lock().unwrap();
        let candidate = recency.iter().copied().find(|p| {
            self.frames
                .read()
                .unwrap()
                .get(p)
                .map(|f| f.pin_count.load(Ordering::Acquire) == 0)
                .unwrap_or(false)
        });
        let Some(page_no) = candidate else {
            return Err(StorageError::BufferPoolFull);
        };
        recency.retain(|p| *p != page_no);
        self.frames.write().unwrap().remove(&page_no);
        Ok(())
    }
}

pub struct PageReadGuard {
    frame: Arc<Frame>,
    page: Page,
}

impl Deref for PageReadGuard {
    type Target = Page;
    fn deref(&self) -> &Page {
        &self.page
    }
}

impl Drop for PageReadGuard {
    fn drop(&mut self) {
        let mut latch = self.frame.latch.lock().unwrap();
        latch.readers -= 1;
        self.frame.changed.notify_all();
        self.frame.pin_count.fetch_sub(1, Ordering::AcqRel);
    }
}

pub struct PageWriteGuard {
    frame: Arc<Frame>,
    page: Page,
}

impl Deref for PageWriteGuard {
    type Target = Page;
    fn deref(&self) -> &Page {
        &self.page
    }
}

impl DerefMut for PageWriteGuard {
    fn deref_mut(&mut self) -> &mut Page {
        &mut self.page
    }
}

impl Drop for PageWriteGuard {
    fn drop(&mut self) {
        *self.frame.page.write().unwrap() = self.page.clone();
        self.frame.dirty.store(true, Ordering::Release);
        let mut latch = self.frame.latch.lock().unwrap();
        latch.writer = false;
        self.frame.changed.notify_all();
        self.frame.pin_count.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::thread;

    #[test]
    fn pinned_frames_cannot_be_evicted() {
        let pool = BufferPool::new(1);
        pool.insert(1, Page::zeroed()).unwrap();
        let guard = pool.fetch(1).unwrap();
        assert!(matches!(
            pool.insert(2, Page::zeroed()),
            Err(StorageError::BufferPoolFull)
        ));
        drop(guard);
        pool.insert(2, Page::zeroed()).unwrap();
        assert!(!pool.contains(1));
    }

    #[test]
    fn writer_waits_for_existing_reader() {
        let pool = Arc::new(BufferPool::new(1));
        pool.insert(1, Page::zeroed()).unwrap();
        let reader = pool.fetch(1).unwrap();
        let acquired = Arc::new(AtomicBool::new(false));
        let pool2 = Arc::clone(&pool);
        let acquired2 = Arc::clone(&acquired);
        let handle = thread::spawn(move || {
            let mut writer = pool2.fetch_mut(1).unwrap();
            acquired2.store(true, Ordering::Release);
            writer.write_u32(0, 42);
        });
        thread::sleep(std::time::Duration::from_millis(25));
        assert!(!acquired.load(Ordering::Acquire));
        drop(reader);
        handle.join().unwrap();
        assert_eq!(pool.fetch(1).unwrap().read_u32(0), 42);
    }

    #[test]
    fn concurrent_readers_share_page() {
        let pool = Arc::new(BufferPool::new(2));
        let mut page = Page::zeroed();
        page.write_u32(0, 12345);
        pool.insert(1, page).unwrap();

        let r1 = pool.fetch(1).unwrap();
        let r2 = pool.fetch(1).unwrap();
        assert_eq!(r1.read_u32(0), 12345);
        assert_eq!(r2.read_u32(0), 12345);
    }
}
