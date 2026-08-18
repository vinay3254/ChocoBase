use crate::error::StorageError;
use crate::storage::page::{Page, PAGE_SIZE};

pub const MAGIC: &[u8; 8] = b"MINIDB\0\x01";

pub struct Header {
    pub page_size: u16,
    pub page_count: u32,
    pub freelist_head: u32,
    pub catalog_root: u32,
}

impl Default for Header {
    fn default() -> Self {
        Self::new()
    }
}

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
        assert!(matches!(
            Header::read_from(&page),
            Err(StorageError::NotADatabase)
        ));
    }
}
