"""Tests for embeddings module (TDD - tests first)."""

import pytest
import numpy as np


class TestEmbeddingService:
    """Test suite for embedding generation."""

    def test_load_model(self):
        """Test loading the embedding model."""
        from src.embeddings import EmbeddingService
        
        service = EmbeddingService(model_name="all-MiniLM-L6-v2")
        
        assert service.model is not None
        assert service.dimension == 384

    def test_embed_single_text(self, sample_text: str):
        """Test embedding a single text string."""
        from src.embeddings import EmbeddingService
        
        service = EmbeddingService()
        embedding = service.embed(sample_text)
        
        assert isinstance(embedding, list)
        assert len(embedding) == 384
        assert all(isinstance(x, float) for x in embedding)

    def test_embed_batch(self):
        """Test batch embedding of multiple texts."""
        from src.embeddings import EmbeddingService
        
        service = EmbeddingService()
        texts = [
            "Machine learning is amazing",
            "Deep learning uses neural networks",
            "VelesDB is fast"
        ]
        
        embeddings = service.embed_batch(texts)
        
        assert len(embeddings) == 3
        assert all(len(e) == 384 for e in embeddings)

    def test_embed_empty_text(self):
        """Test handling of empty text."""
        from src.embeddings import EmbeddingService
        
        service = EmbeddingService()
        
        with pytest.raises(ValueError):
            service.embed("")

    def test_embeddings_are_normalized(self, sample_text: str):
        """Test that embeddings are normalized (unit vectors)."""
        from src.embeddings import EmbeddingService
        
        service = EmbeddingService()
        embedding = service.embed(sample_text)
        
        # Check L2 norm is approximately 1
        norm = np.sqrt(sum(x**2 for x in embedding))
        assert abs(norm - 1.0) < 0.01

    def test_similar_texts_have_similar_embeddings(self):
        """Test that semantically similar texts have similar embeddings."""
        from src.embeddings import EmbeddingService
        
        service = EmbeddingService()
        
        text1 = "Machine learning is a branch of AI"
        text2 = "ML is part of artificial intelligence"
        text3 = "The weather is sunny today"
        
        emb1 = service.embed(text1)
        emb2 = service.embed(text2)
        emb3 = service.embed(text3)
        
        # Cosine similarity
        def cosine_sim(a, b):
            return sum(x*y for x, y in zip(a, b))
        
        sim_12 = cosine_sim(emb1, emb2)
        sim_13 = cosine_sim(emb1, emb3)
        
        # Similar texts should have higher similarity
        assert sim_12 > sim_13
