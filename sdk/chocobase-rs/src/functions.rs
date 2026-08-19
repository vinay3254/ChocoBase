#[derive(Debug, Clone)]
pub struct FunctionsClient {
    pub base_url: String,
    pub api_key: String,
}

impl FunctionsClient {
    pub fn new(base_url: String, api_key: String) -> Self {
        Self {
            base_url: format!("{base_url}/v1/functions"),
            api_key,
        }
    }

    pub fn invoke(&self, function_name: &str, _payload: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "message": format!("Response from {function_name}")
        })
    }
}
