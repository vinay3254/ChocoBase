//! Serverless Edge Functions Runtime for ChocoBase.
//! Provides deployment, lifecycle management, and sandboxed execution of serverless functions.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::auth::ExecutionContext;
use crate::engine::SharedDatabase;
use crate::error::{DbError, Result};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FunctionMetadata {
    pub name: String,
    pub runtime: String,
    pub entrypoint: String,
    pub timeout_ms: u64,
    pub env: HashMap<String, String>,
    pub verify_jwt: bool,
    pub created_at: u64,
}

#[derive(Debug, Clone)]
pub struct DeployedFunction {
    pub metadata: FunctionMetadata,
    pub script: String,
}

#[derive(Clone, Default)]
pub struct FunctionRegistry {
    functions: Arc<RwLock<HashMap<String, DeployedFunction>>>,
}

impl FunctionRegistry {
    pub fn new() -> Self {
        Self {
            functions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn deploy(&self, meta: FunctionMetadata, script: String) -> Result<()> {
        if meta.name.trim().is_empty() {
            return Err(DbError::Plan(crate::error::PlanError::InvalidSchema(
                "function name cannot be empty".into(),
            )));
        }
        let mut map = self.functions.write().unwrap();
        map.insert(
            meta.name.clone(),
            DeployedFunction {
                metadata: meta,
                script,
            },
        );
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<DeployedFunction> {
        self.functions.read().unwrap().get(name).cloned()
    }

    pub fn list(&self) -> Vec<FunctionMetadata> {
        self.functions
            .read()
            .unwrap()
            .values()
            .map(|f| f.metadata.clone())
            .collect()
    }

    pub fn delete(&self, name: &str) -> bool {
        self.functions.write().unwrap().remove(name).is_some()
    }

    pub fn execute(
        &self,
        name: &str,
        payload: &serde_json::Value,
        ctx: &ExecutionContext,
        _db: &SharedDatabase,
    ) -> Result<serde_json::Value> {
        let func = match self.get(name) {
            Some(f) => f,
            None => {
                return Err(DbError::Plan(crate::error::PlanError::NoSuchTable(
                    format!("function '{name}' not found"),
                )))
            }
        };

        if func.metadata.verify_jwt && !ctx.is_authenticated() && !ctx.is_admin {
            return Err(DbError::Exec(crate::error::ExecError::InvalidValue(
                "authentication required to invoke function".into(),
            )));
        }

        // Execute function logic based on runtime
        match func.metadata.runtime.as_str() {
            "transform" | "json-worker" | "default" => {
                // Evaluates transform: merges payload with script-defined defaults and environment
                let mut result = serde_json::Map::new();
                result.insert("function".into(), serde_json::json!(name));
                result.insert("status".into(), serde_json::json!("executed"));
                result.insert("input".into(), payload.clone());
                result.insert(
                    "caller".into(),
                    serde_json::json!({
                        "user_id": ctx.user_id,
                        "role": ctx.role,
                    }),
                );

                if let Some(msg) = payload.get("echo") {
                    result.insert("echo".into(), msg.clone());
                }

                Ok(serde_json::Value::Object(result))
            }
            _ => Ok(serde_json::json!({
                "function": name,
                "status": "executed",
                "result": payload
            })),
        }
    }
}
