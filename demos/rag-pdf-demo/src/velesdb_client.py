"""VelesDB REST API client."""

from typing import Any

import httpx

from .config import get_settings


class VelesDBConnectionError(Exception):
    """Error connecting to VelesDB server."""

    pass


class VelesDBClient:
    """Async client for VelesDB REST API with persistent connection."""

    _client: httpx.AsyncClient | None = None

    def __init__(self, base_url: str | None = None, timeout: float = 30.0):
        settings = get_settings()
        self.base_url = base_url or settings.velesdb_url
        self.timeout = timeout

    async def _get_client(self) -> httpx.AsyncClient:
        """Get or create persistent HTTP client."""
        if VelesDBClient._client is None or VelesDBClient._client.is_closed:
            VelesDBClient._client = httpx.AsyncClient(
                base_url=self.base_url,
                timeout=self.timeout
            )
        return VelesDBClient._client

    async def health_check(self) -> bool:
        """
        Check if VelesDB server is healthy.

        Returns:
            True if healthy, False otherwise

        Raises:
            VelesDBConnectionError: If cannot connect to server
        """
        try:
            client = await self._get_client()
            response = await client.get("/health")
            return response.status_code == 200
        except httpx.ConnectError as e:
            raise VelesDBConnectionError(f"Cannot connect to VelesDB: {e}") from e
        except Exception:
            return False

    async def create_collection(
        self,
        name: str,
        dimension: int,
        metric: str = "cosine"
    ) -> dict[str, Any]:
        """
        Create a new collection.

        Args:
            name: Collection name
            dimension: Vector dimension
            metric: Distance metric (cosine, euclidean, dot)

        Returns:
            Collection info
        """
        client = await self._get_client()
        response = await client.post(
            "/collections",
            json={
                "name": name,
                "dimension": dimension,
                "metric": metric
            }
        )
        response.raise_for_status()
        return response.json()

    async def collection_exists(self, name: str) -> bool:
        """Check if a collection exists."""
        try:
            client = await self._get_client()
            response = await client.get(f"/collections/{name}")
            return response.status_code == 200
        except Exception:
            return False

    async def upsert_points(
        self,
        collection: str,
        points: list[dict[str, Any]]
    ) -> dict[str, Any]:
        """
        Insert or update points in a collection.

        Args:
            collection: Collection name
            points: List of points with id, vector, and payload

        Returns:
            Upsert result
        """
        client = await self._get_client()
        response = await client.post(
            f"/collections/{collection}/points",
            json={"points": points}
        )
        response.raise_for_status()
        return response.json()

    async def search(
        self,
        collection: str,
        query_vector: list[float],
        top_k: int = 10,
        filter_: dict[str, Any] | None = None
    ) -> dict[str, Any]:
        """
        Search for similar vectors.

        Args:
            collection: Collection name
            query_vector: Query vector
            top_k: Number of results
            filter_: Optional metadata filter

        Returns:
            Search results
        """
        payload: dict[str, Any] = {
            "vector": query_vector,
            "top_k": top_k
        }

        if filter_:
            payload["filter"] = filter_

        client = await self._get_client()
        response = await client.post(
            f"/collections/{collection}/search",
            json=payload
        )
        response.raise_for_status()
        return response.json()

    async def delete_by_filter(
        self,
        collection: str,
        filter_: dict[str, Any]
    ) -> dict[str, Any]:
        """
        Delete points matching a filter.

        Args:
            collection: Collection name
            filter_: Metadata filter

        Returns:
            Delete result
        """
        client = await self._get_client()
        response = await client.post(
            f"/collections/{collection}/points/delete",
            json={"filter": filter_}
        )
        response.raise_for_status()
        return response.json()

    async def get_collection_info(self, name: str) -> dict[str, Any]:
        """Get collection information."""
        client = await self._get_client()
        response = await client.get(f"/collections/{name}")
        response.raise_for_status()
        return response.json()
