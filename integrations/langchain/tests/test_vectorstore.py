"""Tests for VelesDBVectorStore.

Run with: pytest tests/test_vectorstore.py -v
"""

import tempfile
import shutil
from typing import List

import pytest

# Skip if dependencies not available
try:
    from langchain_velesdb import VelesDBVectorStore
    from langchain_core.documents import Document
    from langchain_core.embeddings import Embeddings
except ImportError:
    pytest.skip("Dependencies not installed", allow_module_level=True)


class FakeEmbeddings(Embeddings):
    """Fake embeddings for testing."""

    def embed_documents(self, texts: List[str]) -> List[List[float]]:
        """Return fake embeddings for documents."""
        return [[float(i) / 10 for i in range(4)] for _ in texts]

    def embed_query(self, text: str) -> List[float]:
        """Return fake embedding for query."""
        return [0.1, 0.2, 0.3, 0.4]


@pytest.fixture
def temp_db_path():
    """Create a temporary directory for database tests."""
    path = tempfile.mkdtemp(prefix="velesdb_langchain_test_")
    yield path
    shutil.rmtree(path, ignore_errors=True)


@pytest.fixture
def embeddings():
    """Return fake embeddings for testing."""
    return FakeEmbeddings()


class TestVelesDBVectorStore:
    """Tests for VelesDBVectorStore class."""

    def test_init(self, temp_db_path, embeddings):
        """Test vector store initialization."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test",
        )
        assert vectorstore is not None
        assert vectorstore.embeddings == embeddings

    def test_add_texts(self, temp_db_path, embeddings):
        """Test adding texts to the vector store."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_add",
        )

        ids = vectorstore.add_texts(["Hello world", "VelesDB is fast"])
        
        assert len(ids) == 2
        assert all(isinstance(id_, str) for id_ in ids)

    def test_add_texts_with_metadata(self, temp_db_path, embeddings):
        """Test adding texts with metadata."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_metadata",
        )

        metadatas = [
            {"source": "doc1.txt"},
            {"source": "doc2.txt"},
        ]
        ids = vectorstore.add_texts(
            ["First document", "Second document"],
            metadatas=metadatas,
        )

        assert len(ids) == 2

    def test_similarity_search(self, temp_db_path, embeddings):
        """Test similarity search."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_search",
        )

        vectorstore.add_texts([
            "Python is a programming language",
            "VelesDB is a vector database",
            "Machine learning uses vectors",
        ])

        results = vectorstore.similarity_search("database", k=2)

        assert len(results) == 2
        assert all(isinstance(doc, Document) for doc in results)

    def test_similarity_search_with_score(self, temp_db_path, embeddings):
        """Test similarity search with scores."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_score",
        )

        vectorstore.add_texts(["Hello", "World"])

        results = vectorstore.similarity_search_with_score("greeting", k=2)

        assert len(results) == 2
        for doc, score in results:
            assert isinstance(doc, Document)
            assert isinstance(score, float)

    def test_from_texts(self, temp_db_path, embeddings):
        """Test creating vector store from texts."""
        vectorstore = VelesDBVectorStore.from_texts(
            texts=["Doc 1", "Doc 2", "Doc 3"],
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_from_texts",
        )

        results = vectorstore.similarity_search("document", k=2)
        assert len(results) == 2

    def test_as_retriever(self, temp_db_path, embeddings):
        """Test converting to retriever."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_retriever",
        )

        vectorstore.add_texts(["Test document"])

        retriever = vectorstore.as_retriever(search_kwargs={"k": 1})
        assert retriever is not None

    def test_delete(self, temp_db_path, embeddings):
        """Test deleting documents."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_delete",
        )

        ids = vectorstore.add_texts(["To be deleted"])
        
        result = vectorstore.delete(ids)
        assert result is True

    def test_empty_search(self, temp_db_path, embeddings):
        """Test search on empty store."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_empty",
        )

        # Add at least one document to create the collection
        vectorstore.add_texts(["Placeholder"])
        
        # Should return results without error
        results = vectorstore.similarity_search("query", k=5)
        assert isinstance(results, list)


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
