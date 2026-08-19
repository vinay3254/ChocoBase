from typing import Dict, Any, Optional

class StorageClient:
    """Client for Object Storage buckets and files."""

    def __init__(self, url: str, key: str):
        self.url = f"{url}/v1/storage/v1"
        self.key = key

    def from_(self, bucket_id: str) -> "BucketClient":
        return BucketClient(self.url, bucket_id, self.key)

class BucketClient:
    def __init__(self, base_url: str, bucket_id: str, key: str):
        self.url = f"{base_url}/object/{bucket_id}"
        self.bucket_id = bucket_id
        self.key = key

    def upload(self, path: str, file_bytes: bytes, content_type: str = "application/octet-stream") -> Dict[str, Any]:
        return {
            "data": {"path": f"{self.bucket_id}/{path}"},
            "error": None,
        }

    def create_signed_url(self, path: str, expires_in: int = 3600) -> Dict[str, Any]:
        return {
            "data": {
                "signed_url": f"{self.url}/sign/{path}?expires_in={expires_in}"
            },
            "error": None,
        }
