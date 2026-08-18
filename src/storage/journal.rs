//! Rollback Journal implementation for ChocoBase.
//!
//! ### Record Count Finalization Strategy:
//! This implementation uses the **Scan-Until-Checksum-Fail / Scan-Until-EOF** strategy during recovery:
//! - At `BEGIN`, the journal header is written and `fsync`ed with `orig_page_count`.
//! - As pages are mutated, 4104-byte pre-image records (`page_no`, 4096-byte `data`, `crc32`) are appended.
//! - During crash recovery on startup (`recover_if_needed`), the journal is scanned sequentially:
//!   - If a record is valid (correct length and matching CRC32), its pre-image is replayed into the database file.
//!   - If EOF is reached OR a trailing record is truncated/corrupt (simulating a crash mid-record-write),
//!     scanning terminates immediately. All valid pre-images encountered prior to the corruption are retained
//!     and applied, restoring the database to the consistent pre-transaction state without failing recovery.
//!   - Finally, the database file is truncated to `orig_page_count`, `fsync`ed, and the journal is unlinked.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::error::StorageError;
use crate::storage::page::{Page, PAGE_SIZE};

pub const JOURNAL_MAGIC: &[u8; 8] = b"CHOCOJNL";
pub const JOURNAL_VERSION: u16 = 1;
pub const JOURNAL_HEADER_SIZE: usize = 64;
pub const JOURNAL_RECORD_SIZE: usize = 4 + PAGE_SIZE + 4; // 4104 bytes

pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

pub fn journal_path_for(db_path: &Path) -> PathBuf {
    let mut p = db_path.as_os_str().to_os_string();
    p.push("-journal");
    PathBuf::from(p)
}

pub trait JournalWriter: Write + Seek + Send {
    fn sync_all(&mut self) -> std::io::Result<()>;
}

impl JournalWriter for File {
    fn sync_all(&mut self) -> std::io::Result<()> {
        File::sync_all(self)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct JournalHeader {
    pub orig_page_count: u32,
    pub record_count: u32,
}

impl JournalHeader {
    pub fn new(orig_page_count: u32) -> Self {
        JournalHeader {
            orig_page_count,
            record_count: 0,
        }
    }

    pub fn write_to<W: Write + Seek>(&self, file: &mut W) -> Result<(), StorageError> {
        let mut buf = [0u8; JOURNAL_HEADER_SIZE];
        buf[0..8].copy_from_slice(JOURNAL_MAGIC);
        buf[8..10].copy_from_slice(&JOURNAL_VERSION.to_be_bytes());
        buf[10..12].copy_from_slice(&(JOURNAL_HEADER_SIZE as u16).to_be_bytes());
        buf[12..16].copy_from_slice(&self.orig_page_count.to_be_bytes());
        buf[16..20].copy_from_slice(&self.record_count.to_be_bytes());
        let checksum = crc32(&buf[0..20]);
        buf[20..24].copy_from_slice(&checksum.to_be_bytes());

        file.seek(SeekFrom::Start(0))?;
        file.write_all(&buf)?;
        Ok(())
    }

    pub fn read_from<R: Read + Seek>(file: &mut R) -> Result<Self, StorageError> {
        let mut buf = [0u8; JOURNAL_HEADER_SIZE];
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut buf)?;

        if &buf[0..8] != JOURNAL_MAGIC {
            return Err(StorageError::CorruptJournal(
                "invalid journal magic bytes".into(),
            ));
        }
        let version = u16::from_be_bytes(buf[8..10].try_into().unwrap());
        if version != JOURNAL_VERSION {
            return Err(StorageError::CorruptJournal(format!(
                "unsupported journal version {version}"
            )));
        }
        let stored_crc = u32::from_be_bytes(buf[20..24].try_into().unwrap());
        let computed_crc = crc32(&buf[0..20]);
        if stored_crc != computed_crc {
            return Err(StorageError::CorruptJournal(
                "journal header checksum mismatch".into(),
            ));
        }

        let orig_page_count = u32::from_be_bytes(buf[12..16].try_into().unwrap());
        let record_count = u32::from_be_bytes(buf[16..20].try_into().unwrap());

        Ok(JournalHeader {
            orig_page_count,
            record_count,
        })
    }
}

pub struct Journal {
    file: Box<dyn JournalWriter>,
    path: PathBuf,
    header: JournalHeader,
}

impl Journal {
    pub fn create(db_path: &Path, orig_page_count: u32) -> Result<Self, StorageError> {
        let path = journal_path_for(db_path);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;

        Self::new_with_writer(Box::new(file), path, orig_page_count)
    }

    pub fn new_with_writer(
        mut file: Box<dyn JournalWriter>,
        path: PathBuf,
        orig_page_count: u32,
    ) -> Result<Self, StorageError> {
        let header = JournalHeader::new(orig_page_count);
        header.write_to(&mut file)?;
        file.sync_all()?;

        Ok(Journal { file, path, header })
    }

    pub fn append_page(&mut self, page_no: u32, page: &Page) -> Result<(), StorageError> {
        let mut buf = [0u8; JOURNAL_RECORD_SIZE];
        buf[0..4].copy_from_slice(&page_no.to_be_bytes());
        buf[4..4 + PAGE_SIZE].copy_from_slice(&page.data);
        let page_crc = crc32(&page.data);
        buf[4 + PAGE_SIZE..4 + PAGE_SIZE + 4].copy_from_slice(&page_crc.to_be_bytes());

        self.file.seek(SeekFrom::End(0))?;
        self.file.write_all(&buf)?;
        self.header.record_count += 1;
        Ok(())
    }

    pub fn sync(&mut self) -> Result<(), StorageError> {
        self.header.write_to(&mut self.file)?;
        self.file.sync_all()?;
        Ok(())
    }

    pub fn close_and_delete(self) -> Result<(), StorageError> {
        drop(self.file);
        if self.path.exists() {
            fs::remove_file(&self.path)?;
        }
        Ok(())
    }
}

/// Recovers the database by replaying all pre-images from `<db_path>-journal`
/// into the main database file if a journal exists.
///
/// ### Checksum Failure Semantics:
/// - **Stop-at-First-Bad-Record**: Recovery strictly stops scanning at the first corrupt,
///   truncated, or invalid record encountered.
/// - **No-Skipping Policy**: The recovery scanner deliberately does **NOT** attempt to skip
///   past a corrupted record to apply later valid-looking records. Because journal entries
///   represent an ordered sequence of pre-images, skipping a corrupted record risks applying
///   subsequent page restorations while leaving preceding mutated pages unrestored (causing
///   a torn/corrupted database state). Halting at the first bad record guarantees that only
///   the verified prefix is applied, preserving consistency.
pub fn recover_if_needed(db_path: &Path) -> Result<(), StorageError> {
    let jnl_path = journal_path_for(db_path);
    if !jnl_path.exists() {
        return Ok(());
    }

    let metadata = fs::metadata(&jnl_path)?;
    if metadata.len() == 0 {
        // Empty 0-byte leftover journal
        let _ = fs::remove_file(&jnl_path);
        return Ok(());
    }

    let mut jnl_file = match OpenOptions::new().read(true).write(true).open(&jnl_path) {
        Ok(f) => f,
        Err(e) => return Err(StorageError::Io(e)),
    };

    let header = match JournalHeader::read_from(&mut jnl_file) {
        Ok(h) => h,
        Err(e) => return Err(e),
    };

    let mut db_file = OpenOptions::new().read(true).write(true).open(db_path)?;

    // Read all records
    let mut record_buf = [0u8; JOURNAL_RECORD_SIZE];
    jnl_file.seek(SeekFrom::Start(JOURNAL_HEADER_SIZE as u64))?;

    loop {
        match jnl_file.read_exact(&mut record_buf) {
            Ok(()) => {
                let page_no = u32::from_be_bytes(record_buf[0..4].try_into().unwrap());
                let page_data = &record_buf[4..4 + PAGE_SIZE];
                let stored_crc = u32::from_be_bytes(
                    record_buf[4 + PAGE_SIZE..4 + PAGE_SIZE + 4]
                        .try_into()
                        .unwrap(),
                );
                let computed_crc = crc32(page_data);
                if stored_crc != computed_crc {
                    // Semantics: Stop at the first checksum failure. Do not skip to avoid torn state.
                    break;
                }

                // Restore page pre-image into db file
                db_file.seek(SeekFrom::Start(page_no as u64 * PAGE_SIZE as u64))?;
                db_file.write_all(page_data)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                // Clean EOF or truncated trailing record
                break;
            }
            Err(e) => return Err(StorageError::Io(e)),
        }
    }

    // Truncate DB file to original page count
    db_file.set_len(header.orig_page_count as u64 * PAGE_SIZE as u64)?;
    db_file.sync_all()?;

    drop(jnl_file);
    drop(db_file);

    // Remove journal
    let _ = fs::remove_file(&jnl_path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn recovery_restores_valid_prefix_and_ignores_truncated_trailing_record() {
        let db_temp = NamedTempFile::new().unwrap();
        let db_path = db_temp.path();

        // 1. Initialize a 3-page database file
        let mut db_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(db_path)
            .unwrap();
        let page0 = [10u8; PAGE_SIZE];
        let page1 = [20u8; PAGE_SIZE];
        let page2 = [30u8; PAGE_SIZE];
        db_file.write_all(&page0).unwrap();
        db_file.write_all(&page1).unwrap();
        db_file.write_all(&page2).unwrap();
        db_file.sync_all().unwrap();

        // 2. Create a journal with 2 valid records (for page 1 and page 2)
        let jnl_path = journal_path_for(db_path);
        let mut jnl_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&jnl_path)
            .unwrap();

        let header = JournalHeader::new(3);
        header.write_to(&mut jnl_file).unwrap();

        // Write valid record 1 (page 1 pre-image)
        let mut rec1 = [0u8; JOURNAL_RECORD_SIZE];
        rec1[0..4].copy_from_slice(&1u32.to_be_bytes());
        rec1[4..4 + PAGE_SIZE].copy_from_slice(&page1);
        let crc1 = crc32(&page1);
        rec1[4 + PAGE_SIZE..4 + PAGE_SIZE + 4].copy_from_slice(&crc1.to_be_bytes());
        jnl_file.write_all(&rec1).unwrap();

        // Write valid record 2 (page 2 pre-image)
        let mut rec2 = [0u8; JOURNAL_RECORD_SIZE];
        rec2[0..4].copy_from_slice(&2u32.to_be_bytes());
        rec2[4..4 + PAGE_SIZE].copy_from_slice(&page2);
        let crc2 = crc32(&page2);
        rec2[4 + PAGE_SIZE..4 + PAGE_SIZE + 4].copy_from_slice(&crc2.to_be_bytes());
        jnl_file.write_all(&rec2).unwrap();

        // 3. Write an incomplete/truncated 3rd record (e.g. 500 bytes instead of 4104)
        let truncated_bytes = [0xFFu8; 500];
        jnl_file.write_all(&truncated_bytes).unwrap();
        jnl_file.sync_all().unwrap();
        drop(jnl_file);

        // Mutate db_file to simulate corrupted/modified state
        db_file.seek(SeekFrom::Start(1 * PAGE_SIZE as u64)).unwrap();
        db_file.write_all(&[99u8; PAGE_SIZE]).unwrap();
        db_file.seek(SeekFrom::Start(2 * PAGE_SIZE as u64)).unwrap();
        db_file.write_all(&[88u8; PAGE_SIZE]).unwrap();
        db_file.sync_all().unwrap();
        drop(db_file);

        // 4. Run recovery
        recover_if_needed(db_path).unwrap();

        // 5. Verify pre-images for page 1 and page 2 were restored
        let mut restored_db = File::open(db_path).unwrap();
        let mut buf1 = [0u8; PAGE_SIZE];
        restored_db
            .seek(SeekFrom::Start(1 * PAGE_SIZE as u64))
            .unwrap();
        restored_db.read_exact(&mut buf1).unwrap();
        assert_eq!(buf1, page1);

        let mut buf2 = [0u8; PAGE_SIZE];
        restored_db
            .seek(SeekFrom::Start(2 * PAGE_SIZE as u64))
            .unwrap();
        restored_db.read_exact(&mut buf2).unwrap();
        assert_eq!(buf2, page2);

        // Journal should be deleted
        assert!(!jnl_path.exists());
    }
}
