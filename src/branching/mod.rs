//! Database Branching and Ephemeral Staging Environments for ChocoBase.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::sync::RwLock;

use crate::engine::SharedDatabase;
use crate::error::{DbError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchMetadata {
    pub name: String,
    pub created_at_ms: u64,
    pub path: String,
    pub size_bytes: u64,
}

pub struct BranchManager {
    base_dir: PathBuf,
    branches: RwLock<HashMap<String, (BranchMetadata, SharedDatabase)>>,
}

impl BranchManager {
    pub fn new(base_dir: impl AsRef<Path>) -> Self {
        let path = base_dir.as_ref().to_path_buf();
        let _ = std::fs::create_dir_all(&path);
        Self {
            base_dir: path,
            branches: RwLock::new(HashMap::new()),
        }
    }

    pub async fn create_branch(
        &self,
        name: &str,
        source_db: &SharedDatabase,
    ) -> Result<BranchMetadata> {
        let sanitized_name = name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect::<String>();

        if sanitized_name.is_empty() {
            return Err(DbError::Plan(crate::error::PlanError::InvalidExpression(
                "branch name cannot be empty".into(),
            )));
        }

        let mut map = self.branches.write().await;
        if map.contains_key(&sanitized_name) {
            return Err(DbError::Plan(crate::error::PlanError::TableAlreadyExists(
                format!("branch '{sanitized_name}' already exists"),
            )));
        }

        // Dump source database SQL and restore to new branch file
        let branch_file = self.base_dir.join(format!("{sanitized_name}.db"));
        if branch_file.exists() {
            let _ = std::fs::remove_file(&branch_file);
        }

        let dump_sql = source_db.with_db(crate::backup::dump_database)?;

        let branch_db = SharedDatabase::create(&branch_file)?;
        branch_db.with_db(|db| {
            crate::backup::restore_database(db, &dump_sql)?;
            Ok(())
        })?;

        let size_bytes = std::fs::metadata(&branch_file)
            .map(|m| m.len())
            .unwrap_or(0);
        let created_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let meta = BranchMetadata {
            name: sanitized_name.clone(),
            created_at_ms,
            path: branch_file.to_string_lossy().to_string(),
            size_bytes,
        };

        map.insert(sanitized_name, (meta.clone(), branch_db));
        Ok(meta)
    }

    pub async fn list_branches(&self) -> Vec<BranchMetadata> {
        let map = self.branches.read().await;
        map.values().map(|(meta, _)| meta.clone()).collect()
    }

    pub async fn get_branch_db(&self, name: &str) -> Option<SharedDatabase> {
        let map = self.branches.read().await;
        map.get(name).map(|(_, db)| db.clone())
    }

    pub async fn delete_branch(&self, name: &str) -> bool {
        let mut map = self.branches.write().await;
        if let Some((meta, _)) = map.remove(name) {
            let _ = std::fs::remove_file(&meta.path);
            true
        } else {
            false
        }
    }
}
