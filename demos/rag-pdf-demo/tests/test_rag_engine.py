"""Tests for the RAG engine module.

The engine and its VelesDB client are fully synchronous (the demo uses the
embedded ``velesdb`` Python bindings, not a REST server), so every test below
mocks the collaborators with plain ``MagicMock`` and asserts the real
synchronous return shapes.
"""

from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest


class TestRAGEngine:
    """Test suite for RAG orchestration."""

    def test_ingest_document(self, sample_pdf_path: Path):
        """Ingesting a real PDF embeds its chunks and upserts them to VelesDB."""
        from src.rag_engine import RAGEngine

        with patch("src.rag_engine.VelesDBClient") as mock_velesdb, \
             patch("src.rag_engine.EmbeddingService") as mock_embeddings:

            # The client is synchronous: methods return values directly.
            mock_velesdb_instance = MagicMock()
            # Collection already exists -> create_collection is skipped.
            mock_velesdb_instance.collection_exists.return_value = True
            # Empty collection -> _load_existing_documents() returns early.
            mock_velesdb_instance.get_collection_info.return_value = {
                "point_count": 0
            }
            mock_velesdb_instance.upsert_points.return_value = {"upserted": 5}
            mock_velesdb.return_value = mock_velesdb_instance

            mock_embeddings_instance = MagicMock()
            # Plenty of embeddings; zip() truncates to the real chunk count.
            mock_embeddings_instance.embed_batch.return_value = [[0.1] * 384] * 16
            mock_embeddings_instance.dimension = 384
            mock_embeddings.return_value = mock_embeddings_instance

            engine = RAGEngine()
            result = engine.ingest_document(sample_pdf_path)

            assert result["success"] is True
            assert result["document_name"] == sample_pdf_path.name
            assert result["chunks_created"] > 0
            assert result["pages_processed"] == 2
            # Existing collection -> no create, but points were upserted.
            mock_velesdb_instance.create_collection.assert_not_called()
            mock_velesdb_instance.upsert_points.assert_called_once()
            # The document is now tracked in the local registry.
            assert sample_pdf_path.name in engine._documents

    def test_search_documents(self):
        """Search embeds the query and reformats the client's raw results."""
        from src.rag_engine import RAGEngine

        with patch("src.rag_engine.VelesDBClient") as mock_velesdb, \
             patch("src.rag_engine.EmbeddingService") as mock_embeddings:

            mock_velesdb_instance = MagicMock()
            # Client.search returns {"results": [{"id", "score", "payload"}]}.
            mock_velesdb_instance.search.return_value = {
                "results": [
                    {
                        "id": 1,
                        "score": 0.95,
                        "payload": {
                            "text": "Machine learning is AI",
                            "document_name": "test.pdf",
                            "page_number": 1,
                        },
                    }
                ]
            }
            mock_velesdb.return_value = mock_velesdb_instance

            mock_embeddings_instance = MagicMock()
            mock_embeddings_instance.embed.return_value = [0.1] * 384
            mock_embeddings.return_value = mock_embeddings_instance

            engine = RAGEngine()
            results = engine.search("What is machine learning?", top_k=5)

            # Engine returns formatted results plus timing metrics.
            assert len(results["results"]) == 1
            hit = results["results"][0]
            assert abs(hit["score"] - 0.95) < 1e-6
            assert hit["text"] == "Machine learning is AI"
            assert hit["document_name"] == "test.pdf"
            assert hit["page_number"] == 1
            assert "embedding_time_ms" in results
            assert "search_time_ms" in results
            # No document filter -> client.search called with filter_=None.
            _, kwargs = mock_velesdb_instance.search.call_args
            assert kwargs["filter_"] is None
            assert kwargs["top_k"] == 5

    def test_search_documents_with_filter(self):
        """A document_filter is translated into an eq metadata filter."""
        from src.rag_engine import RAGEngine

        with patch("src.rag_engine.VelesDBClient") as mock_velesdb, \
             patch("src.rag_engine.EmbeddingService") as mock_embeddings:

            mock_velesdb_instance = MagicMock()
            mock_velesdb_instance.search.return_value = {"results": []}
            mock_velesdb.return_value = mock_velesdb_instance

            mock_embeddings_instance = MagicMock()
            mock_embeddings_instance.embed.return_value = [0.1] * 384
            mock_embeddings.return_value = mock_embeddings_instance

            engine = RAGEngine()
            results = engine.search(
                "query", top_k=3, document_filter="report.pdf"
            )

            assert results["results"] == []
            _, kwargs = mock_velesdb_instance.search.call_args
            assert kwargs["filter_"] == {
                "condition": {
                    "type": "eq",
                    "field": "document_name",
                    "value": "report.pdf",
                }
            }

    def test_get_documents_list(self):
        """list_documents returns the values of the local registry."""
        from src.rag_engine import RAGEngine

        with patch("src.rag_engine.VelesDBClient"), \
             patch("src.rag_engine.EmbeddingService"):
            engine = RAGEngine()
            # empty store
            assert engine.list_documents() == []
            # seed registry
            engine._documents["test.pdf"] = {
                "name": "test.pdf",
                "pages": 2,
                "chunks": 3,
                "chunk_ids": [1, 2, 3],
            }
            docs = engine.list_documents()
            assert isinstance(docs, list)
            assert len(docs) == 1
            assert docs[0]["name"] == "test.pdf"
            assert docs[0]["chunks"] == 3

    def test_delete_document(self):
        """Deleting a document deletes each tracked chunk via delete_point."""
        from src.rag_engine import RAGEngine

        with patch("src.rag_engine.VelesDBClient") as mock_velesdb, \
             patch("src.rag_engine.EmbeddingService") as mock_embeddings:

            mock_velesdb_instance = MagicMock()
            # delete_point is called once per chunk id.
            mock_velesdb_instance.delete_point.return_value = {"deleted": 1}
            mock_velesdb.return_value = mock_velesdb_instance

            mock_embeddings_instance = MagicMock()
            mock_embeddings_instance.dimension = 384
            mock_embeddings.return_value = mock_embeddings_instance

            engine = RAGEngine()
            # Seed the registry with a document and its chunk ids.
            engine._documents["test.pdf"] = {
                "name": "test.pdf",
                "pages": 1,
                "chunks": 3,
                "chunk_ids": [123, 456, 789],
            }

            result = engine.delete_document("test.pdf")

            # All three chunks deleted, one delete_point call each.
            assert result["deleted"] == 3
            assert mock_velesdb_instance.delete_point.call_count == 3
            assert result["message"] == "Deleted 3/3 chunks"
            assert "errors" not in result
            # Document removed from the registry.
            assert "test.pdf" not in engine._documents

    def test_delete_nonexistent_document(self):
        """Deleting an unknown document is a no-op returning 0 deleted."""
        from src.rag_engine import RAGEngine

        with patch("src.rag_engine.VelesDBClient") as mock_velesdb, \
             patch("src.rag_engine.EmbeddingService") as mock_embeddings:

            mock_velesdb_instance = MagicMock()
            mock_velesdb.return_value = mock_velesdb_instance

            mock_embeddings_instance = MagicMock()
            mock_embeddings_instance.dimension = 384
            mock_embeddings.return_value = mock_embeddings_instance

            engine = RAGEngine()
            result = engine.delete_document("nonexistent.pdf")

            # Unknown document -> 0 deleted, no client call.
            assert result["deleted"] == 0
            assert result["message"] == "Document not found"
            mock_velesdb_instance.delete_point.assert_not_called()


class TestRAGEngineIntegration:
    """Integration tests (require a populated VelesDB instance)."""

    @pytest.mark.skip(reason="Requires running VelesDB server")
    def test_full_rag_pipeline(self, sample_pdf_path: Path):
        """Test complete RAG pipeline with real VelesDB."""
        from src.rag_engine import RAGEngine

        engine = RAGEngine()

        # Ingest
        ingest_result = engine.ingest_document(sample_pdf_path)
        assert ingest_result["success"] is True

        # Search
        search_results = engine.search("What is machine learning?")
        assert len(search_results["results"]) > 0

        # Delete
        delete_result = engine.delete_document(sample_pdf_path.name)
        assert delete_result["deleted"] > 0
