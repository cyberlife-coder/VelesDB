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

from typing import Any, Protocol, Sequence, runtime_checkable

try:  # optional backend, gated by `pip install velesdb[embed-openai]`
    import openai as _openai
except ImportError:  # pragma: no cover - tested via no-extras install
    _openai = None  # type: ignore[assignment]

try:  # optional backend, gated by `pip install velesdb[embed-sentence-transformers]`
    import sentence_transformers as _sentence_transformers
except ImportError:  # pragma: no cover - tested via no-extras install
    _sentence_transformers = None  # type: ignore[assignment]

_OPENAI_MISSING_HINT = (
    "OpenAIEmbedder requires the 'openai' package. "
    "Install with: pip install velesdb[embed-openai]"
)
_SENTENCE_TRANSFORMERS_MISSING_HINT = (
    "SentenceTransformerEmbedder requires 'sentence-transformers'. "
    "Install with: pip install velesdb[embed-sentence-transformers]"
)


@runtime_checkable
class Embedder(Protocol):
    """Minimal interface a VelesDB embedding adapter must satisfy.

    ``dimension`` is ``0`` until it can be inferred — either by passing it
    explicitly to the adapter constructor or by calling :meth:`embed` once.
    """

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
        openai_module = _load_openai()
        self._client = openai_module.OpenAI(api_key=api_key, base_url=base_url)
        self.model = model
        self.dimension: int = dimensions or 0

    def embed(self, texts: Sequence[str]) -> list[list[float]]:
        if not texts:
            return []
        kwargs: dict[str, Any] = {"model": self.model, "input": list(texts)}
        if self.dimension > 0:
            kwargs["dimensions"] = self.dimension
        response = self._client.embeddings.create(**kwargs)
        vectors = [list(item.embedding) for item in response.data]
        if not self.dimension and vectors:
            self.dimension = len(vectors[0])
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
        sentence_transformers_module = _load_sentence_transformers()
        self._model = sentence_transformers_module.SentenceTransformer(model, device=device)
        self._normalize = normalize
        dim = self._model.get_sentence_embedding_dimension()
        self.dimension: int = int(dim) if dim is not None else 0

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


def _load_openai() -> Any:
    if _openai is None:
        raise ImportError(_OPENAI_MISSING_HINT)
    return _openai


def _load_sentence_transformers() -> Any:
    if _sentence_transformers is None:
        raise ImportError(_SENTENCE_TRANSFORMERS_MISSING_HINT)
    return _sentence_transformers


__all__ = [
    "Embedder",
    "OpenAIEmbedder",
    "SentenceTransformerEmbedder",
]
