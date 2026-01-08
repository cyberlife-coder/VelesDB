"""Embedding service using sentence-transformers."""

from sentence_transformers import SentenceTransformer

from .config import get_settings


class EmbeddingService:
    """Generate embeddings using sentence-transformers (free, local)."""

    def __init__(self, model_name: str | None = None):
        settings = get_settings()
        self.model_name = model_name or settings.embedding_model
        self._model: SentenceTransformer | None = None
        self._dimension: int | None = None

    @property
    def model(self) -> SentenceTransformer:
        """Lazy load the model."""
        if self._model is None:
            self._model = SentenceTransformer(self.model_name)
        return self._model

    @property
    def dimension(self) -> int:
        """Get embedding dimension."""
        if self._dimension is None:
            # Get dimension from model
            self._dimension = self.model.get_sentence_embedding_dimension()
        return self._dimension

    def embed(self, text: str) -> list[float]:
        """
        Generate embedding for a single text.

        Args:
            text: Text to embed

        Returns:
            List of floats representing the embedding

        Raises:
            ValueError: If text is empty
        """
        if not text or not text.strip():
            raise ValueError("Text cannot be empty")

        embedding = self.model.encode(
            text,
            normalize_embeddings=True,
            convert_to_numpy=True
        )

        return embedding.tolist()

    def embed_batch(self, texts: list[str]) -> list[list[float]]:
        """
        Generate embeddings for multiple texts.

        Args:
            texts: List of texts to embed

        Returns:
            List of embeddings (same length as input, empty texts get zero vector)
        """
        if not texts:
            return []

        results = []
        valid_indices = []
        valid_texts = []
        
        # Track which texts are valid
        for i, t in enumerate(texts):
            if t and t.strip():
                valid_indices.append(i)
                valid_texts.append(t)
        
        if not valid_texts:
            # All texts were empty, return zero vectors
            return [[0.0] * self.dimension for _ in texts]

        # Generate embeddings for valid texts
        embeddings = self.model.encode(
            valid_texts,
            normalize_embeddings=True,
            convert_to_numpy=True,
            show_progress_bar=len(valid_texts) > 10
        )
        
        # Map back to original indices, fill empty texts with zero vector
        embedding_map = {idx: emb.tolist() for idx, emb in zip(valid_indices, embeddings)}
        zero_vector = [0.0] * self.dimension
        
        for i in range(len(texts)):
            if i in embedding_map:
                results.append(embedding_map[i])
            else:
                results.append(zero_vector)

        return results
