"""Maximal marginal relevance (MMR) and by-vector search operations.

Implements the standard MMR selection in pure Python (no numpy dependency):
candidates are greedily picked by ``lambda_mult * sim(query, candidate)
- (1 - lambda_mult) * max(sim(candidate, selected))`` until *k* documents
are selected. Candidate vectors are fetched from VelesDB via
``collection.get`` after the initial dense search.
"""

from __future__ import annotations

import math
from typing import Any, List, Optional, Sequence

from langchain_core.documents import Document

from langchain_velesdb.security import validate_k, validate_text


def cosine_similarity(a: Sequence[float], b: Sequence[float]) -> float:
    """Return the cosine similarity of two vectors (0.0 on zero norm)."""
    dot = 0.0
    norm_a = 0.0
    norm_b = 0.0
    for x, y in zip(a, b):
        dot += x * y
        norm_a += x * x
        norm_b += y * y
    if norm_a == 0.0 or norm_b == 0.0:
        return 0.0
    return dot / math.sqrt(norm_a * norm_b)


def _best_mmr_candidate(
    candidates: List[Sequence[float]],
    query_sims: List[float],
    selected: List[int],
    lambda_mult: float,
) -> Optional[int]:
    """Return the index of the unselected candidate with the best MMR score."""
    best_idx: Optional[int] = None
    best_score = -math.inf
    for i, candidate in enumerate(candidates):
        if i in selected:
            continue
        redundancy = max(
            (cosine_similarity(candidate, candidates[j]) for j in selected),
            default=0.0,
        )
        score = lambda_mult * query_sims[i] - (1.0 - lambda_mult) * redundancy
        if score > best_score:
            best_idx = i
            best_score = score
    return best_idx


def mmr_select(
    query_embedding: Sequence[float],
    candidate_embeddings: List[Sequence[float]],
    k: int,
    lambda_mult: float = 0.5,
) -> List[int]:
    """Return candidate indices selected by maximal marginal relevance.

    Args:
        query_embedding: Dense query vector.
        candidate_embeddings: Candidate vectors, in retrieval order.
        k: Maximum number of candidates to select.
        lambda_mult: Diversity factor in ``[0, 1]`` — 1 means pure
            relevance, 0 means maximum diversity.

    Returns:
        List of selected indices into *candidate_embeddings*, in
        selection order.
    """
    if k <= 0 or not candidate_embeddings:
        return []
    query_sims = [cosine_similarity(query_embedding, c) for c in candidate_embeddings]
    selected: List[int] = []
    while len(selected) < min(k, len(candidate_embeddings)):
        best_idx = _best_mmr_candidate(
            candidate_embeddings, query_sims, selected, lambda_mult
        )
        if best_idx is None:
            break
        selected.append(best_idx)
    return selected


class MMRSearchMixin:
    """Mixin providing by-vector and MMR search for VelesDBVectorStore.

    Expects the host class to provide ``self._embedding``,
    ``self._collection``, ``self._run_vector_search(...)`` and
    ``self._to_document(result)``.
    """

    def similarity_search_by_vector(
        self,
        embedding: List[float],
        k: int = 4,
        filter: Optional[dict] = None,
        **kwargs: Any,
    ) -> List[Document]:
        """Search for documents most similar to a raw embedding vector.

        Args:
            embedding: Dense query vector.
            k: Number of results to return. Defaults to 4.
            filter: Optional metadata filter dict (VelesDB filter format).
            **kwargs: Additional arguments.

        Returns:
            List of Documents most similar to the embedding.
        """
        validate_k(k)
        results = self._run_vector_search(list(embedding), k, filter=filter)
        return [self._to_document(result) for result in results]

    def max_marginal_relevance_search(
        self,
        query: str,
        k: int = 4,
        fetch_k: int = 20,
        lambda_mult: float = 0.5,
        filter: Optional[dict] = None,
        **kwargs: Any,
    ) -> List[Document]:
        """Return documents selected using maximal marginal relevance.

        MMR optimises for similarity to the query AND diversity among the
        selected documents.

        Args:
            query: Text to look up documents similar to.
            k: Number of Documents to return. Defaults to 4.
            fetch_k: Number of candidates fetched for the MMR pass.
                Defaults to 20.
            lambda_mult: Diversity factor in ``[0, 1]`` — 1 means pure
                relevance, 0 means maximum diversity. Defaults to 0.5.
            filter: Optional metadata filter dict (VelesDB filter format).
            **kwargs: Additional arguments.

        Returns:
            List of Documents selected by maximal marginal relevance.
        """
        validate_text(query)
        query_embedding = self._embedding.embed_query(query)
        return self.max_marginal_relevance_search_by_vector(
            query_embedding, k=k, fetch_k=fetch_k,
            lambda_mult=lambda_mult, filter=filter, **kwargs,
        )

    def max_marginal_relevance_search_by_vector(
        self,
        embedding: List[float],
        k: int = 4,
        fetch_k: int = 20,
        lambda_mult: float = 0.5,
        filter: Optional[dict] = None,
        **kwargs: Any,
    ) -> List[Document]:
        """Return documents selected by MMR for a raw embedding vector.

        Args: same as :meth:`max_marginal_relevance_search`, with
        *embedding* replacing *query*.

        Returns:
            List of Documents selected by maximal marginal relevance.
        """
        validate_k(k)
        validate_k(fetch_k, "fetch_k")
        results = self._run_vector_search(list(embedding), fetch_k, filter=filter)
        candidates = self._candidates_with_vectors(results)
        selected = mmr_select(
            embedding, [vector for _, vector in candidates], k, lambda_mult
        )
        return [self._to_document(candidates[i][0]) for i in selected]

    def _candidates_with_vectors(self, results: List[dict]) -> List[tuple]:
        """Pair search results with their stored vectors (skips missing)."""
        ids = [result["id"] for result in results]
        points = self._collection.get(ids)
        vectors_by_id = {
            point["id"]: point["vector"] for point in points if point is not None
        }
        return [
            (result, vectors_by_id[result["id"]])
            for result in results
            if result["id"] in vectors_by_id
        ]
