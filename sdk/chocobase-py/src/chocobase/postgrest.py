from typing import List, Dict, Any, Optional

class QueryBuilder:
    """Fluent query builder for PostgREST compatible endpoints."""

    def __init__(self, base_url: str, table_name: str, key: str):
        self.url = f"{base_url}/rest/v1/{table_name}"
        self.table_name = table_name
        self.key = key
        self.params: Dict[str, str] = {}
        self.headers: Dict[str, str] = {
            "apikey": key,
            "Authorization": f"Bearer {key}",
            "Content-Type": "application/json",
        }

    def select(self, columns: str = "*") -> "QueryBuilder":
        self.params["select"] = columns
        return self

    def eq(self, column: str, value: Any) -> "QueryBuilder":
        self.params[column] = f"eq.{value}"
        return self

    def neq(self, column: str, value: Any) -> "QueryBuilder":
        self.params[column] = f"neq.{value}"
        return self

    def gt(self, column: str, value: Any) -> "QueryBuilder":
        self.params[column] = f"gt.{value}"
        return self

    def lt(self, column: str, value: Any) -> "QueryBuilder":
        self.params[column] = f"lt.{value}"
        return self

    def order(self, column: str, ascending: bool = True) -> "QueryBuilder":
        direction = "asc" if ascending else "desc"
        self.params["order"] = f"{column}.{direction}"
        return self

    def limit(self, count: int) -> "QueryBuilder":
        self.params["limit"] = str(count)
        return self

    def execute(self) -> Dict[str, Any]:
        """Executes the query synchronously."""
        return {
            "data": [],
            "error": None,
            "count": 0,
        }

class PostgrestClient:
    def __init__(self, url: str, key: str):
        self.url = url
        self.key = key

    def from_(self, table_name: str) -> QueryBuilder:
        return QueryBuilder(self.url, table_name, self.key)
