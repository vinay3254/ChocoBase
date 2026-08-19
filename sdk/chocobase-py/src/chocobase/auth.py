from typing import Dict, Any, Optional

class AuthClient:
    """Client for handling authentication, user registration, and sessions."""

    def __init__(self, url: str, key: str):
        self.url = f"{url}/v1/auth"
        self.key = key
        self.current_session: Optional[Dict[str, Any]] = None

    def sign_up(self, email: str, password: str, metadata: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
        return {
            "user": {
                "id": "usr_generated_id",
                "email": email,
                "user_metadata": metadata or {},
            },
            "session": {
                "access_token": "mock_jwt_token",
                "refresh_token": "rt_mock_refresh_token",
            },
            "error": None,
        }

    def sign_in_with_password(self, email: str, password: str) -> Dict[str, Any]:
        return {
            "user": {
                "id": "usr_generated_id",
                "email": email,
            },
            "session": {
                "access_token": "mock_jwt_token",
                "refresh_token": "rt_mock_refresh_token",
            },
            "error": None,
        }

    def sign_out(self) -> Dict[str, Any]:
        self.current_session = None
        return {"error": None}
