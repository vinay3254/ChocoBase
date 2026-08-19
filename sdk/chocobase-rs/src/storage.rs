#[derive(Debug, Clone)]
pub struct BucketClient {
    pub base_url: String,
    pub bucket: String,
    pub api_key: String,
}

impl BucketClient {
    pub fn create_signed_url(&self, path: &str, expires_in: u64) -> String {
        format!("{}/sign/{}?expires_in={expires_in}", self.base_url, path)
    }
}

#[derive(Debug, Clone)]
pub struct StorageClient {
    pub base_url: String,
    pub api_key: String,
}

impl StorageClient {
    pub fn new(base_url: String, api_key: String) -> Self {
        Self {
            base_url: format!("{base_url}/v1/storage/v1"),
            api_key,
        }
    }

    pub fn from(&self, bucket: &str) -> BucketClient {
        BucketClient {
            base_url: format!("{}/object/{bucket}", self.base_url),
            bucket: bucket.to_string(),
            api_key: self.api_key.clone(),
        }
    }
}
