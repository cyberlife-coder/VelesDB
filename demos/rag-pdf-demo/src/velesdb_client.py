"""VelesDB native client — wraps the velesdb Python bindings directly."""

import tempfile
from typing import Any

import velesdb


class VelesDBClient:
    """Synchronous VelesDB client backed by native Python bindings.

    Replaces the previous httpx-based REST client. All calls go directly
    into the embedded Rust engine — no network hop, no server process.

    The GIL is released during every Rust call (search, upsert, delete),
    so CPU-bound work does not block other threads.
    """

    def __init__(
        self,
        data_path: str | None = None,
        *,
        _base_url: str | None = None,
        _timeout: float | None = None,
    ) -> None:
        """Open a VelesDB database at *data_path*.

        Args:
            data_path: Directory for persistent vector storage.
                If ``None``, a per-process temporary directory is used.
            _base_url: Ignored — kept for backward-compat call sites that
                pass ``base_url=`` as a keyword.
            _timeout: Ignored — native calls have no network timeout.
        """
        if data_path is None:
            # Create a temp dir that lives for the lifetime of this object.
            self._tmpdir = tempfile.TemporaryDirectory()
            data_path = self._tmpdir.name
        else:
            self._tmpdir = None

        self._db = velesdb.Database(data_path)

    # ------------------------------------------------------------------
    # Collection management
    # ------------------------------------------------------------------

    def create_collection(
        self,
        name: str,
        dimension: int,
        metric: str = "cosine",
    ) -> dict[str, Any]:
        """Create a new vector collection.

        Returns:
            Info dict with collection metadata.

        Raises:
            velesdb.CollectionExistsError: If the collection already exists.
        """
        col = self._db.create_collection(name, dimension=dimension, metric=metric)
        return col.info()

    def collection_exists(self, name: str) -> bool:
        """Return True if a collection with *name* already exists."""
        return self._db.get_collection(name) is not None

    def get_collection_info(self, name: str) -> dict[str, Any]:
        """Return collection metadata dict.

        Raises:
            velesdb.CollectionNotFoundError: If the collection does not exist.
        """
        col = self._db.get_collection(name)
        if col is None:
            raise velesdb.CollectionNotFoundError(
                f"Collection '{name}' not found"
            )
        return col.info()

    def delete_collection(self, name: str) -> dict[str, Any]:
        """Drop an entire collection.

        Returns:
            Result dict with ``{"deleted": name}``.
        """
        self._db.drop_collection(name)
        return {"deleted": name}

    # ------------------------------------------------------------------
    # Point operations
    # ------------------------------------------------------------------

    def upsert_points(
        self,
        collection: str,
        points: list[dict[str, Any]],
    ) -> dict[str, Any]:
        """Insert or update *points* in *collection*.

        Args:
            collection: Collection name.
            points: List of dicts with ``id`` (int), ``vector`` (list[float]),
                and optional ``payload`` (dict).

        Returns:
            Result dict with ``{"upserted": <count>}``.
        """
        col = self._db.get_collection(collection)
        if col is None:
            raise velesdb.CollectionNotFoundError(
                f"Collection '{collection}' not found"
            )
        count = col.upsert(points)
        return {"upserted": count}

    def search(
        self,
        collection: str,
        query_vector: list[float],
        top_k: int = 10,
        filter_: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        """Search for similar vectors in *collection*.

        Args:
            collection: Collection name.
            query_vector: Dense query embedding.
            top_k: Maximum number of results.
            filter_: Optional metadata filter dict.

        Returns:
            Dict with ``{"results": [{"id": ..., "score": ..., "payload": ...}]}``.
        """
        col = self._db.get_collection(collection)
        if col is None:
            raise velesdb.CollectionNotFoundError(
                f"Collection '{collection}' not found"
            )
        results = col.search(vector=query_vector, top_k=top_k, filter=filter_)
        return {"results": results}

    def delete_point(
        self,
        collection: str,
        point_id: int,
    ) -> dict[str, Any]:
        """Delete a single point by ID.

        Returns:
            Dict with ``{"deleted": point_id}``.
        """
        col = self._db.get_collection(collection)
        if col is None:
            raise velesdb.CollectionNotFoundError(
                f"Collection '{collection}' not found"
            )
        col.delete([point_id])
        return {"deleted": point_id}

    def health_check(self) -> bool:
        """Return True if the database is operational."""
        try:
            self._db.list_collections()
            return True
        except Exception:
            return False
