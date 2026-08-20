//! Multi-Project Fleet Control Plane and Tenant Lifecycle Engine for ChocoBase.
//! Manages organizations, project provisioning, dynamic credentials, resource quotas, and usage metering.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Organization {
    pub id: String,
    pub name: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectStatus {
    Provisioning,
    Active,
    Paused,
    Deleting,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProjectQuota {
    pub max_storage_mb: u64,
    pub max_egress_mb: u64,
    pub max_realtime_connections: u64,
    pub max_functions: u64,
}

impl Default for ProjectQuota {
    fn default() -> Self {
        Self {
            max_storage_mb: 1024,
            max_egress_mb: 5120,
            max_realtime_connections: 500,
            max_functions: 50,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ResourceUsage {
    pub storage_bytes: u64,
    pub egress_bytes: u64,
    pub realtime_connections: u64,
    pub function_invocations: u64,
    pub total_rows: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Project {
    pub id: String,
    pub org_id: String,
    pub name: String,
    pub region: String,
    pub status: ProjectStatus,
    pub anon_key: String,
    pub service_role_key: String,
    pub database_path: String,
    pub quota: ProjectQuota,
    pub usage: ResourceUsage,
    pub created_at_ms: u64,
}

pub struct ControlPlane {
    orgs: Arc<Mutex<HashMap<String, Organization>>>,
    projects: Arc<Mutex<HashMap<String, Project>>>,
}

static CONTROL_PLANE: OnceLock<ControlPlane> = OnceLock::new();

impl ControlPlane {
    pub fn global() -> &'static ControlPlane {
        CONTROL_PLANE.get_or_init(|| {
            let mut orgs = HashMap::new();
            let mut projects = HashMap::new();

            let default_org = Organization {
                id: "org_default".to_string(),
                name: "Default Organization".to_string(),
                created_at_ms: now_ms(),
            };

            let default_project = Project {
                id: "prj_default".to_string(),
                org_id: "org_default".to_string(),
                name: "Production Workspace".to_string(),
                region: "us-east-1".to_string(),
                status: ProjectStatus::Active,
                anon_key: "anon_key_production_default".to_string(),
                service_role_key: "service_role_key_production_default".to_string(),
                database_path: "chocobase.db".to_string(),
                quota: ProjectQuota::default(),
                usage: ResourceUsage {
                    storage_bytes: 1048576,
                    egress_bytes: 2097152,
                    realtime_connections: 12,
                    function_invocations: 148,
                    total_rows: 450,
                },
                created_at_ms: now_ms(),
            };

            orgs.insert(default_org.id.clone(), default_org);
            projects.insert(default_project.id.clone(), default_project);

            ControlPlane {
                orgs: Arc::new(Mutex::new(orgs)),
                projects: Arc::new(Mutex::new(projects)),
            }
        })
    }

    pub fn create_organization(&self, name: &str) -> Organization {
        let id = format!("org_{}", generate_id());
        let org = Organization {
            id: id.clone(),
            name: name.to_string(),
            created_at_ms: now_ms(),
        };
        let mut map = self.orgs.lock().unwrap();
        map.insert(id, org.clone());
        org
    }

    pub fn list_organizations(&self) -> Vec<Organization> {
        let map = self.orgs.lock().unwrap();
        map.values().cloned().collect()
    }

    pub fn create_project(&self, org_id: &str, name: &str, region: &str) -> Result<Project, String> {
        let orgs = self.orgs.lock().unwrap();
        if !orgs.contains_key(org_id) {
            return Err(format!("organization '{org_id}' not found"));
        }
        drop(orgs);

        let id = format!("prj_{}", generate_id());
        let anon_key = format!("anon_{}", generate_id());
        let service_role_key = format!("service_role_{}", generate_id());
        let database_path = format!("{id}.db");

        let project = Project {
            id: id.clone(),
            org_id: org_id.to_string(),
            name: name.to_string(),
            region: region.to_string(),
            status: ProjectStatus::Active,
            anon_key,
            service_role_key,
            database_path,
            quota: ProjectQuota::default(),
            usage: ResourceUsage {
                storage_bytes: 0,
                egress_bytes: 0,
                realtime_connections: 0,
                function_invocations: 0,
                total_rows: 0,
            },
            created_at_ms: now_ms(),
        };

        let mut map = self.projects.lock().unwrap();
        map.insert(id, project.clone());
        Ok(project)
    }

    pub fn list_projects(&self) -> Vec<Project> {
        let map = self.projects.lock().unwrap();
        map.values().cloned().collect()
    }

    pub fn get_project(&self, project_id: &str) -> Option<Project> {
        let map = self.projects.lock().unwrap();
        map.get(project_id).cloned()
    }

    pub fn resolve_project(&self, identifier: &str) -> Option<Project> {
        let map = self.projects.lock().unwrap();
        if let Some(p) = map.get(identifier) {
            return Some(p.clone());
        }
        for p in map.values() {
            if p.anon_key == identifier || p.service_role_key == identifier || p.id == identifier {
                return Some(p.clone());
            }
        }
        None
    }

    pub fn pause_project(&self, project_id: &str) -> Result<Project, String> {
        let mut map = self.projects.lock().unwrap();
        if let Some(p) = map.get_mut(project_id) {
            p.status = ProjectStatus::Paused;
            Ok(p.clone())
        } else {
            Err(format!("project '{project_id}' not found"))
        }
    }

    pub fn resume_project(&self, project_id: &str) -> Result<Project, String> {
        let mut map = self.projects.lock().unwrap();
        if let Some(p) = map.get_mut(project_id) {
            p.status = ProjectStatus::Active;
            Ok(p.clone())
        } else {
            Err(format!("project '{project_id}' not found"))
        }
    }

    pub fn record_egress(&self, project_id: &str, bytes: u64) {
        let mut map = self.projects.lock().unwrap();
        if let Some(p) = map.get_mut(project_id) {
            p.usage.egress_bytes += bytes;
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn generate_id() -> String {
    let mut buf = [0u8; 8];
    let _ = getrandom::getrandom(&mut buf);
    let mut s = String::new();
    for b in buf {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
