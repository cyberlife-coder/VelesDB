"""Collection administration mixin shared across VelesDB Python integrations.

Provides ``create_metadata_collection``, ``is_metadata_only``, and
``train_pq`` — three methods whose logic is identical in both the
LangChain and LlamaIndex adapters.

Host classes must expose:
    - ``self._get_db()`` returning a ``velesdb.Database`` instance.
    - ``self._collection`` — the active collection or ``None``.
    - ``self.collection_name`` — the current collection name string.
"""

from __future__ import annotations

from typing import Any, Dict, Optional


class CollectionAdminMixin:
    """Mixin providing collection-level admin operations for VelesDB adapters.

    Expects the host class to provide:
        - ``self._get_db()`` — returns or creates the active database.
        - ``self._collection`` — the active ``VectorCollection`` or ``None``.
        - ``self.collection_name`` — the name of the active collection.
    """

    def create_metadata_collection(self, name: str) -> None:
        """Create a metadata-only collection (no vectors).

        Useful for storing reference data that can be JOINed with
        vector collections (VelesDB Premium feature).

        Args:
            name: Collection name.
        """
        db = self._get_db()
        db.create_metadata_collection(name)

    def is_metadata_only(self) -> bool:
        """Check if the current collection is metadata-only.

        Returns:
            True if metadata-only, False if vector collection.
        """
        if self._collection is None:
            return False
        return self._collection.is_metadata_only()

    def train_pq(self, m: int = 8, k: int = 256, opq: bool = False) -> Any:
        """Train Product Quantization on the collection.

        PQ training is a Database-level operation (not Collection-level)
        because TRAIN QUANTIZER requires Database-level VelesQL execution.

        Args:
            m: Number of subspaces. Defaults to 8.
            k: Number of centroids per subspace. Defaults to 256.
            opq: Enable Optimized PQ pre-rotation. Defaults to False.

        Returns:
            Training result message.
        """
        return self._get_db().train_pq(self.collection_name, m=m, k=k, opq=opq)

    def analyze_collection(self) -> dict:
        """Analyze the collection, computing and persisting statistics.

        Delegates to ``Database.analyze_collection(name)`` which computes
        per-column stats including histogram metadata.

        Returns:
            Dict with keys: ``total_points``, ``row_count``, ``deleted_count``,
            ``avg_row_size_bytes``, ``payload_size_bytes``, ``column_stats``
            (dict mapping column names to per-column stat dicts with optional
            ``histogram_buckets`` and ``histogram_stale`` fields).
        """
        return self._get_db().analyze_collection(self.collection_name)

    def get_collection_stats(self) -> Optional[Dict[str, Any]]:
        """Get cached collection statistics, or None if never analyzed.

        Delegates to ``Database.get_collection_stats(name)``.

        Returns:
            Dict with same structure as ``analyze_collection()`` return value,
            or None if the collection has never been analyzed.
        """
        return self._get_db().get_collection_stats(self.collection_name)
