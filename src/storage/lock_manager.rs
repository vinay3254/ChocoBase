//! Strict Two-Phase Locking (2PL) Lock Manager for ChocoBase.
//!
//! Provides shared/exclusive locking over database and table resources.
//! Locks are held for the duration of a transaction and automatically released
//! upon commit, rollback, or token drop.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockMode {
    Shared,
    Exclusive,
}

#[derive(Default)]
struct State {
    owners: HashMap<String, Vec<(u64, LockMode)>>,
}

pub struct LockManager {
    state: Mutex<State>,
    changed: Condvar,
    next_tx: AtomicU64,
}

impl LockManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            next_tx: AtomicU64::new(1),
        })
    }

    pub fn begin(self: &Arc<Self>) -> LockToken {
        LockToken {
            manager: Arc::clone(self),
            tx_id: self.next_tx.fetch_add(1, Ordering::Relaxed),
            held: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn acquire(&self, tx_id: u64, resource: &str, mode: LockMode) {
        let _ = self.acquire_timeout(tx_id, resource, mode, std::time::Duration::from_secs(30));
    }

    pub fn acquire_timeout(
        &self,
        tx_id: u64,
        resource: &str,
        mode: LockMode,
        timeout: std::time::Duration,
    ) -> Result<(), crate::error::StorageError> {
        let start = std::time::Instant::now();
        let mut state = self.state.lock().unwrap();
        loop {
            let owners = state.owners.get(resource).cloned().unwrap_or_default();
            let compatible = owners.iter().all(|(owner, held)| {
                *owner == tx_id || matches!((mode, *held), (LockMode::Shared, LockMode::Shared))
            });
            if compatible {
                state
                    .owners
                    .entry(resource.to_string())
                    .or_default()
                    .push((tx_id, mode));
                return Ok(());
            }

            let elapsed = start.elapsed();
            if elapsed >= timeout {
                return Err(crate::error::StorageError::LockTimeout);
            }
            let remaining = timeout - elapsed;
            let (new_state, timeout_res) = self.changed.wait_timeout(state, remaining).unwrap();
            state = new_state;
            if timeout_res.timed_out() {
                let owners_check = state.owners.get(resource).cloned().unwrap_or_default();
                let compatible_check = owners_check.iter().all(|(owner, held)| {
                    *owner == tx_id || matches!((mode, *held), (LockMode::Shared, LockMode::Shared))
                });
                if compatible_check {
                    state
                        .owners
                        .entry(resource.to_string())
                        .or_default()
                        .push((tx_id, mode));
                    return Ok(());
                }
                return Err(crate::error::StorageError::LockTimeout);
            }
        }
    }

    fn release_all(&self, tx_id: u64) {
        let mut state = self.state.lock().unwrap();
        for owners in state.owners.values_mut() {
            owners.retain(|(owner, _)| *owner != tx_id);
        }
        state.owners.retain(|_, owners| !owners.is_empty());
        self.changed.notify_all();
    }
}

pub struct LockToken {
    manager: Arc<LockManager>,
    tx_id: u64,
    held: Arc<Mutex<Vec<String>>>,
}

impl Clone for LockToken {
    fn clone(&self) -> Self {
        Self {
            manager: Arc::clone(&self.manager),
            tx_id: self.tx_id,
            held: Arc::clone(&self.held),
        }
    }
}

impl LockToken {
    pub fn id(&self) -> u64 {
        self.tx_id
    }

    pub fn shared(&self, resource: &str) {
        self.manager.acquire(self.tx_id, resource, LockMode::Shared);
        self.held.lock().unwrap().push(resource.to_string());
    }

    pub fn try_shared(
        &self,
        resource: &str,
        timeout: std::time::Duration,
    ) -> Result<(), crate::error::StorageError> {
        self.manager
            .acquire_timeout(self.tx_id, resource, LockMode::Shared, timeout)?;
        self.held.lock().unwrap().push(resource.to_string());
        Ok(())
    }

    pub fn exclusive(&self, resource: &str) {
        self.manager
            .acquire(self.tx_id, resource, LockMode::Exclusive);
        self.held.lock().unwrap().push(resource.to_string());
    }

    pub fn try_exclusive(
        &self,
        resource: &str,
        timeout: std::time::Duration,
    ) -> Result<(), crate::error::StorageError> {
        self.manager
            .acquire_timeout(self.tx_id, resource, LockMode::Exclusive, timeout)?;
        self.held.lock().unwrap().push(resource.to_string());
        Ok(())
    }
}

impl Drop for LockToken {
    fn drop(&mut self) {
        if Arc::strong_count(&self.held) == 1 {
            self.manager.release_all(self.tx_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use std::thread;

    #[test]
    fn shared_locks_overlap_and_writer_waits() {
        let lm = LockManager::new();
        let a = lm.begin();
        let b = lm.begin();
        let writer = lm.begin();
        a.shared("db");
        b.shared("db");

        let barrier = Arc::new(Barrier::new(2));
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let lm2 = Arc::clone(&lm);
        let d2 = Arc::clone(&done);
        let br = Arc::clone(&barrier);

        let t = thread::spawn(move || {
            let w = lm2.begin();
            br.wait();
            w.exclusive("db");
            d2.store(true, Ordering::Release);
        });

        barrier.wait();
        thread::yield_now();
        assert!(!done.load(Ordering::Acquire));
        drop(a);
        drop(b);
        t.join().unwrap();
        assert!(done.load(Ordering::Acquire));
        drop(writer);
    }

    #[test]
    fn exclusive_locks_serialize_and_release_on_drop() {
        let lm = LockManager::new();
        let a = lm.begin();
        a.exclusive("table_users");

        let acquired = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let lm2 = Arc::clone(&lm);
        let ac2 = Arc::clone(&acquired);

        let handle = thread::spawn(move || {
            let b = lm2.begin();
            b.exclusive("table_users");
            ac2.store(true, Ordering::Release);
        });

        thread::sleep(std::time::Duration::from_millis(25));
        assert!(!acquired.load(Ordering::Acquire));
        drop(a);
        handle.join().unwrap();
        assert!(acquired.load(Ordering::Acquire));
    }

    #[test]
    fn conflicting_lock_times_out_and_avoids_deadlock() {
        let lm = LockManager::new();
        let a = lm.begin();
        a.exclusive("table_orders");

        let b = lm.begin();
        let res = b.try_exclusive("table_orders", std::time::Duration::from_millis(50));
        assert!(matches!(res, Err(crate::error::StorageError::LockTimeout)));

        drop(a);
        let res_after = b.try_exclusive("table_orders", std::time::Duration::from_millis(50));
        assert!(res_after.is_ok());
    }
}
