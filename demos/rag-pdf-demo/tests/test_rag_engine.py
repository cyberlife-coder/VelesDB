"""Tests for RAG engine module (TDD - tests first)."""

import pytest
from unittest.mock import AsyncMock, MagicMock, patch
from pathlib import Path


class TestRAGEngine:
    """Test suite for RAG orchestration."""

    @pytest.mark.asyncio
    async def test_ingest_document(self, sample_pdf_path: Path):
        """Test document ingestion pipeline."""
        from src.rag_engine import RAGEngine
        
        with patch("src.rag_engine.VelesDBClient") as mock_velesdb, \
             patch("src.rag_engine.EmbeddingService") as mock_embeddings:
            
            # Setup mocks - all async methods need AsyncMock
            mock_velesdb_instance = MagicMock()
            mock_velesdb_instance.collection_exists = AsyncMock(return_value=True)
            mock_velesdb_instance.create_collection = AsyncMock()
            mock_velesdb_instance.upsert_points = AsyncMock(return_value={"inserted": 5})
            mock_velesdb.return_value = mock_velesdb_instance
            
            mock_embeddings_instance = MagicMock()
            mock_embeddings_instance.embed_batch.return_value = [[0.1] * 384] * 5
            mock_embeddings_instance.dimension = 384
            mock_embeddings.return_value = mock_embeddings_instance
            
            engine = RAGEngine()
            result = await engine.ingest_document(sample_pdf_path)
            
            assert result["success"] is True
            assert result["chunks_created"] > 0

    @pytest.mark.asyncio
    async def test_search_documents(self):
        """Test document search."""
        from src.rag_engine import RAGEngine
        
        with patch("src.rag_engine.VelesDBClient") as mock_velesdb, \
             patch("src.rag_engine.EmbeddingService") as mock_embeddings:
            
            # Setup mocks
            mock_velesdb_instance = MagicMock()
            mock_velesdb_instance.search = AsyncMock(return_value={
                "results": [
                    {
                        "id": 1,
                        "score": 0.95,
                        "payload": {
                            "text": "Machine learning is AI",
                            "document_name": "test.pdf",
                            "page_number": 1
                        }
                    }
                ]
            })
            mock_velesdb.return_value = mock_velesdb_instance
            
            mock_embeddings_instance = MagicMock()
            mock_embeddings_instance.embed.return_value = [0.1] * 384
            mock_embeddings.return_value = mock_embeddings_instance
            
            engine = RAGEngine()
            results = await engine.search("What is machine learning?", top_k=5)
            
            assert len(results) > 0
            assert results[0]["score"] == 0.95
            assert "Machine learning" in results[0]["text"]

    @pytest.mark.asyncio
    async def test_get_documents_list(self):
        """Test listing indexed documents."""
        from src.rag_engine import RAGEngine
        
        engine = RAGEngine()
        # This should work even with empty store
        documents = await engine.list_documents()
        
        assert isinstance(documents, list)

    @pytest.mark.asyncio
    async def test_delete_document(self):
        """Test document deletion."""
        from src.rag_engine import RAGEngine
        
        with patch("src.rag_engine.VelesDBClient") as mock_velesdb:
            mock_velesdb_instance = MagicMock()
            mock_velesdb_instance.delete_by_filter = AsyncMock(return_value={"deleted": 5})
            mock_velesdb.return_value = mock_velesdb_instance
            
            engine = RAGEngine()
            result = await engine.delete_document("test.pdf")
            
            assert result["deleted"] > 0


class TestRAGEngineIntegration:
    """Integration tests (require VelesDB server)."""

    @pytest.mark.skip(reason="Requires running VelesDB server")
    @pytest.mark.asyncio
    async def test_full_rag_pipeline(self, sample_pdf_path: Path):
        """Test complete RAG pipeline with real VelesDB."""
        from src.rag_engine import RAGEngine
        
        engine = RAGEngine()
        
        # Ingest
        ingest_result = await engine.ingest_document(sample_pdf_path)
        assert ingest_result["success"] is True
        
        # Search
        search_results = await engine.search("What is machine learning?")
        assert len(search_results) > 0
        
        # Delete
        delete_result = await engine.delete_document(sample_pdf_path.name)
        assert delete_result["deleted"] > 0
