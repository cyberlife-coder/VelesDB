"""Optional embedding helpers for VelesDB.

A thin, dependency-free :class:`Embedder` protocol plus a handful of
adapters around common providers. Each adapter lazy-imports its backend
so plain ``import velesdb`` stays zero-dep beyond NumPy.

Install the matching extra to enable an adapter::

    pip install velesdb[embed-openai]
    pip install velesdb[embed-sentence-transformers]
    pip install velesdb[embed]   # both

Example::

    from velesdb import Database
    from velesdb.embed import SentenceTransformerEmbedder

    embedder = SentenceTransformerEmbedder("all-MiniLM-L6-v2")
    db = Database("./data")
    db.create_collection("docs", dimension=embedder.dimension)
    vectors = embedder.embed(["hello world", "vector search rocks"])
"""

from __future__ import annotations

from typing import Protocol, Sequence, runtime_checkable


@runtime_checkable
class Embedder(Protocol):
    """Minimal interface a VelesDB embedding adapter must satisfy."""

    dimension: int

    def embed(self, texts: Sequence[str]) -> list[list[float]]:  # pragma: no cover - protocol
        ...


class OpenAIEmbedder:
    """OpenAI / Azure-OpenAI compatible embedding adapter.

    Requires ``pip install velesdb[embed-openai]``. The ``base_url`` argument
    lets you point the same client at Azure OpenAI, vLLM, or any other
    OpenAI-compatible endpoint.
    """

    def __init__(
        self,
        model: str = "text-embedding-3-small",
        *,
        api_key: str | None = None,
        base_url: str | None = None,
        dimensions: int | None = None,
    ) -> None:
        try:
            from openai import OpenAI
        except ImportError as exc:
            raise ImportError(
                "OpenAIEmbedder requires the 'openai' package. "
                "Install with: pip install velesdb[embed-openai]"
            ) from exc

        self._client = OpenAI(api_key=api_key, base_url=base_url)
        self.model = model
        self._dimension = dimensions or 0

    @property
    def dimension(self) -> int:
        if self._dimension == 0:
            # Probe once so callers can rely on ``embedder.dimension`` before
            # the first real embed call (e.g. when sizing a collection).
            self.embed(["dimension-probe"])
        return self._dimension

    def embed(self, texts: Sequence[str]) -> list[list[float]]:
        if not texts:
            return []
        kwargs: dict[str, object] = {"model": self.model, "input": list(texts)}
        if self._dimension > 0:
            kwargs["dimensions"] = self._dimension
        response = self._client.embeddings.create(**kwargs)
        vectors = [list(item.embedding) for item in response.data]
        if self._dimension == 0 and vectors:
            self._dimension = len(vectors[0])
        return vectors


class SentenceTransformerEmbedder:
    """Local SentenceTransformers adapter — no API key, runs on-device.

    Requires ``pip install velesdb[embed-sentence-transformers]``.
    """

    def __init__(
        self,
        model: str = "all-MiniLM-L6-v2",
        *,
        device: str | None = None,
        normalize: bool = True,
    ) -> None:
        try:
            from sentence_transformers import SentenceTransformer
        except ImportError as exc:
            raise ImportError(
                "SentenceTransformerEmbedder requires 'sentence-transformers'. "
                "Install with: pip install velesdb[embed-sentence-transformers]"
            ) from exc

        self._model = SentenceTransformer(model, device=device)
        self._normalize = normalize
        self.dimension = int(self._model.get_sentence_embedding_dimension())

    def embed(self, texts: Sequence[str]) -> list[list[float]]:
        if not texts:
            return []
        vectors = self._model.encode(
            list(texts),
            convert_to_numpy=True,
            show_progress_bar=False,
            normalize_embeddings=self._normalize,
        )
        return vectors.tolist()


__all__ = [
    "Embedder",
    "OpenAIEmbedder",
    "SentenceTransformerEmbedder",
]
