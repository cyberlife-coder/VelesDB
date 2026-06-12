"""E2E tests for the documented Quick Start and delete(ref_doc_id) semantics.

These tests guard the two documented contracts:

- the README Quick Start (StorageContext + from_documents) actually
  persists chunks into the VelesDB collection;
- delete(ref_doc_id) removes EVERY chunk of a multi-chunk document.

Run with: pytest tests/test_quickstart_e2e.py -v
"""

import shutil
import tempfile

import pytest

try:
    from llama_index.core import (
        Document,
        SimpleDirectoryReader,
        StorageContext,
        VectorStoreIndex,
    )
    from llama_index.core.embeddings.mock_embed_model import MockEmbedding
    from llama_index.core.node_parser import SentenceSplitter
    from llamaindex_velesdb import VelesDBVectorStore
except ImportError:
    pytest.skip("Dependencies not installed", allow_module_level=True)

_DIM = 8


@pytest.fixture
def temp_dir():
    path = tempfile.mkdtemp(prefix="velesdb_llamaindex_quickstart_")
    yield path
    shutil.rmtree(path, ignore_errors=True)


@pytest.fixture
def data_dir():
    path = tempfile.mkdtemp(prefix="velesdb_quickstart_data_")
    with open(f"{path}/about.txt", "w", encoding="utf-8") as handle:
        handle.write(
            "VelesDB is a local-first database unifying vector, graph and "
            "column storage under a single query language called VelesQL."
        )
    yield path
    shutil.rmtree(path, ignore_errors=True)


class TestQuickstart:
    """The README Quick Start must write chunks into VelesDB."""

    def test_quickstart_persists_chunks_in_velesdb(self, temp_dir, data_dir):
        """Quick Start flow, verbatim (MockEmbedding instead of a paid API)."""
        # Create vector store
        vector_store = VelesDBVectorStore(
            path=temp_dir,
            collection_name="my_docs",
            metric="cosine",
        )

        # Wrap it in a StorageContext (required, see README)
        storage_context = StorageContext.from_defaults(vector_store=vector_store)

        # Load and index documents — chunks are written to VelesDB
        documents = SimpleDirectoryReader(data_dir).load_data()
        VectorStoreIndex.from_documents(
            documents,
            storage_context=storage_context,
            embed_model=MockEmbedding(embed_dim=_DIM),
        )

        # THEN: the VelesDB collection contains the indexed chunks
        assert not vector_store.is_empty()
        info = vector_store.get_collection_info()
        assert info["point_count"] >= len(documents)

        # And the chunks are retrievable through the store
        nodes, _cursor = vector_store.scroll(batch_size=10)
        assert any("VelesDB" in node.get_content() for node in nodes)


class TestDeleteByRefDocId:
    """delete(ref_doc_id) must remove every chunk of the document."""

    def _chunks_for(self, text: str, doc_id: str):
        document = Document(text=text, doc_id=doc_id)
        splitter = SentenceSplitter(chunk_size=16, chunk_overlap=0)
        nodes = splitter.get_nodes_from_documents([document])
        for node in nodes:
            node.embedding = [float(len(node.get_content()) % 7 + 1)] * _DIM
        return nodes

    def test_delete_removes_all_chunks_of_document(self, temp_dir):
        store = VelesDBVectorStore(path=temp_dir, collection_name="del_chunks")

        parent_chunks = self._chunks_for(
            " ".join(
                f"Sentence number {i} describes part {i} of the system in detail."
                for i in range(6)
            ),
            doc_id="parent_doc",
        )
        assert len(parent_chunks) >= 3, "test needs a multi-chunk document"
        for chunk in parent_chunks:
            assert chunk.ref_doc_id == "parent_doc"

        other_chunks = self._chunks_for(
            "Unrelated content that must survive.", doc_id="other_doc"
        )

        store.add(parent_chunks + other_chunks)
        total = len(parent_chunks) + len(other_chunks)
        assert store.get_collection_info()["point_count"] == total

        # WHEN: the parent document is deleted by its ref_doc_id
        store.delete("parent_doc")

        # THEN: zero parent chunks remain; unrelated chunks survive
        assert store.get_collection_info()["point_count"] == len(other_chunks)
        remaining, _cursor = store.scroll(batch_size=50)
        assert all(
            node.metadata.get("ref_doc_id") != "parent_doc" for node in remaining
        )
        assert len(remaining) == len(other_chunks)

    def test_delete_handles_more_chunks_than_velesql_default_limit(self, temp_dir):
        """VelesQL SELECT defaults to LIMIT 10 — delete must not stop there."""
        store = VelesDBVectorStore(path=temp_dir, collection_name="del_many")

        chunks = self._chunks_for(
            " ".join(f"Sentence number {i} talks about topic {i}." for i in range(40)),
            doc_id="big_doc",
        )
        assert len(chunks) > 10
        store.add(chunks)

        store.delete("big_doc")

        assert store.get_collection_info()["point_count"] == 0

    def test_delete_on_fresh_store_binds_existing_collection(self, temp_dir):
        """A new store instance over existing data can delete immediately."""
        writer = VelesDBVectorStore(path=temp_dir, collection_name="del_fresh")
        writer.add(self._chunks_for("Some persisted sentences. More text.", "doc_x"))
        writer.flush()
        count = writer.get_collection_info()["point_count"]
        assert count > 0
        del writer

        reader = VelesDBVectorStore(path=temp_dir, collection_name="del_fresh")
        reader.delete("doc_x")
        assert reader.get_collection_info()["point_count"] == 0
