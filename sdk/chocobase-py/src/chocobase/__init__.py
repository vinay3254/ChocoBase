"""ChocoBase Python Client SDK"""

from .client import ChocoClient, create_client
from .auth import AuthClient
from .postgrest import PostgrestClient, QueryBuilder
from .storage import StorageClient
from .functions import FunctionsClient
from .realtime import RealtimeClient

__version__ = "0.1.0"
__all__ = [
    "ChocoClient",
    "create_client",
    "AuthClient",
    "PostgrestClient",
    "QueryBuilder",
    "StorageClient",
    "FunctionsClient",
    "RealtimeClient",
]
