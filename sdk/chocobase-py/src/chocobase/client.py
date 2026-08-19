from typing import Optional
from .auth import AuthClient
from .postgrest import PostgrestClient, QueryBuilder
from .storage import StorageClient
from .functions import FunctionsClient
from .realtime import RealtimeClient

class ChocoClient:
    """Main client class for interacting with ChocoBase platform."""

    def __init__(self, url: str, key: str, options: Optional[dict] = None):
        self.url = url.rstrip('/')
        self.key = key
        self.options = options or {}

        self.auth = AuthClient(self.url, self.key)
        self.postgrest = PostgrestClient(self.url, self.key)
        self.storage = StorageClient(self.url, self.key)
        self.functions = FunctionsClient(self.url, self.key)
        self.realtime = RealtimeClient(self.url, self.key)

    def from_(self, table_name: str) -> QueryBuilder:
        """Shorthand to start a PostgREST query builder on a table."""
        return self.postgrest.from_(table_name)

    def table(self, table_name: str) -> QueryBuilder:
        """Alias for from_."""
        return self.from_(table_name)

def create_client(url: str, key: str, options: Optional[dict] = None) -> ChocoClient:
    """Creates a new ChocoBase client instance."""
    return ChocoClient(url, key, options)
