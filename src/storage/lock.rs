//! Advisory file locking for single-writer concurrency guarding in ChocoBase.
//!
//! ### Concurrency Model & Lock Lifecycle:
//! - When opening/creating a database file (`Database::open` or `Database::create`), an exclusive
//!   advisory lock file `<database>.lock` is created containing the current process PID.
//! - If another process attempts to open the database while an active process holds the lock,
//!   `StorageError::DatabaseLocked` is returned.
//! - When the `Database` (and its `LockFile`) is dropped, the `.lock` file is cleanly unlinked.
//!
//! ### Stale Lock Reclamation:
//! - If `<database>.lock` exists from a previous crash or unclean shutdown, `LockFile::acquire`
//!   reads the recorded PID and queries the OS (`GetExitCodeProcess` / `STILL_ACTIVE` on Windows,
//!   `kill(pid, 0)` on Unix) to determine if that PID is still running.
//! - If the process is dead, the stale lock is automatically reclaimed for the current process.
//!
//! ### Known Limitations:
//! - **PID Reuse Risk:** PID-based liveness checks are inherently vulnerable to OS PID reuse if
//!   an unrelated new process is spawned with the exact same PID as a crashed database process before
//!   the database is reopened. This is acceptable for single-machine embedded use, but should be
//!   replaced by OS-level file locking primitives (e.g. `flock` / `LockFileEx`) if multi-process
//!   or server-mode usage is introduced in the future.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::error::StorageError;

pub fn lock_path_for(db_path: &Path) -> PathBuf {
    let mut p = db_path.as_os_str().to_os_string();
    p.push(".lock");
    PathBuf::from(p)
}

#[cfg(windows)]
pub fn is_pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    unsafe {
        use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle == 0 {
            return false;
        }
        let mut exit_code = 0u32;
        let success = GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);
        if success == 0 {
            return false;
        }
        exit_code == STILL_ACTIVE as u32
    }
}

#[cfg(unix)]
pub fn is_pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[cfg(not(any(windows, unix)))]
pub fn is_pid_alive(_pid: u32) -> bool {
    false
}

#[derive(Debug)]
pub struct LockFile {
    path: PathBuf,
    pid: u32,
}

impl LockFile {
    pub fn acquire(db_path: &Path) -> Result<Self, StorageError> {
        let lock_path = lock_path_for(db_path);
        let my_pid = std::process::id();

        if lock_path.exists() {
            if let Ok(mut file) = OpenOptions::new().read(true).open(&lock_path) {
                let mut contents = String::new();
                if file.read_to_string(&mut contents).is_ok() {
                    if let Ok(existing_pid) = contents.trim().parse::<u32>() {
                        if existing_pid != my_pid && is_pid_alive(existing_pid) {
                            return Err(StorageError::DatabaseLocked(format!(
                                "database is locked by active process PID {existing_pid}"
                            )));
                        }
                    }
                }
            }
            // Stale lock from dead process or corrupt lock file: safely reclaim
            let _ = fs::remove_file(&lock_path);
        }

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&lock_path)?;
        file.write_all(my_pid.to_string().as_bytes())?;
        file.sync_all()?;

        Ok(LockFile {
            path: lock_path,
            pid: my_pid,
        })
    }
}

impl Drop for LockFile {
    fn drop(&mut self) {
        if self.path.exists() {
            // Only remove if it contains our PID
            if let Ok(mut file) = File::open(&self.path) {
                let mut contents = String::new();
                if file.read_to_string(&mut contents).is_ok() {
                    if let Ok(p) = contents.trim().parse::<u32>() {
                        if p == self.pid {
                            let _ = fs::remove_file(&self.path);
                        }
                    }
                }
            }
        }
    }
}
