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

from typing import Any


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
