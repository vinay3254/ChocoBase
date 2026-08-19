use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct QueryBuilder {
    pub base_url: String,
    pub table: String,
    pub api_key: String,
    pub params: HashMap<String, String>,
}

impl QueryBuilder {
    pub fn select(mut self, columns: &str) -> Self {
        self.params.insert("select".to_string(), columns.to_string());
        self
    }

    pub fn eq(mut self, column: &str, value: impl ToString) -> Self {
        self.params.insert(column.to_string(), format!("eq.{}", value.to_string()));
        self
    }

    pub fn neq(mut self, column: &str, value: impl ToString) -> Self {
        self.params.insert(column.to_string(), format!("neq.{}", value.to_string()));
        self
    }

    pub fn gt(mut self, column: &str, value: impl ToString) -> Self {
        self.params.insert(column.to_string(), format!("gt.{}", value.to_string()));
        self
    }

    pub fn lt(mut self, column: &str, value: impl ToString) -> Self {
        self.params.insert(column.to_string(), format!("lt.{}", value.to_string()));
        self
    }

    pub fn limit(mut self, count: usize) -> Self {
        self.params.insert("limit".to_string(), count.to_string());
        self
    }

    pub fn execute(&self) -> serde_json::Value {
        serde_json::json!({
            "data": [],
            "count": 0,
            "error": null
        })
    }
}

#[derive(Debug, Clone)]
pub struct PostgrestClient {
    pub base_url: String,
    pub api_key: String,
}

impl PostgrestClient {
    pub fn new(base_url: String, api_key: String) -> Self {
        Self { base_url, api_key }
    }

    pub fn from(&self, table: &str) -> QueryBuilder {
        QueryBuilder {
            base_url: format!("{}/rest/v1/{table}", self.base_url),
            table: table.to_string(),
            api_key: self.api_key.clone(),
            params: HashMap::new(),
        }
    }
}
