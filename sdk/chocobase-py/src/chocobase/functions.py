from typing import Dict, Any, Optional

class FunctionsClient:
    """Client for invoking serverless Edge Functions."""

    def __init__(self, url: str, key: str):
        self.url = f"{url}/v1/functions"
        self.key = key

    def invoke(self, function_name: str, options: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
        return {
            "data": {"message": f"Response from {function_name}"},
            "error": None,
        }
