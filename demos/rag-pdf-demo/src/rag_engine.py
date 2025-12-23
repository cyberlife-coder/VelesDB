"""RAG Engine - orchestrates PDF processing, embedding, and search."""

import time
from datetime import datetime
from pathlib import Path
from typing import Any

from .config import get_settings
from .embeddings import EmbeddingService
from .pdf_processor import PDFProcessor
from .velesdb_client import VelesDBClient


class RAGEngine:
    """Main RAG orchestration engine."""

    def __init__(self):
        self.settings = get_settings()
        self.pdf_processor = PDFProcessor()
        self.embedding_service = EmbeddingService()
        self.velesdb = VelesDBClient()
        self._documents: dict[str, dict[str, Any]] = {}

    async def ensure_collection(self) -> None:
        """Ensure the RAG collection exists in VelesDB."""
        collection_name = self.settings.collection_name

        if not await self.velesdb.collection_exists(collection_name):
            await self.velesdb.create_collection(
                name=collection_name,
                dimension=self.embedding_service.dimension,
                metric="cosine"
            )

    async def ingest_document(self, pdf_path: Path) -> dict[str, Any]:
        """
        Ingest a PDF document into VelesDB.

        Args:
            pdf_path: Path to the PDF file

        Returns:
            Ingestion result with stats
        """
        await self.ensure_collection()

        # Process PDF into chunks (with timing)
        t0 = time.perf_counter()
        chunks = self.pdf_processor.process_pdf(pdf_path)
        processing_time_ms = (time.perf_counter() - t0) * 1000

        if not chunks:
            return {
                "success": False,
                "document_name": pdf_path.name,
                "pages_processed": 0,
                "chunks_created": 0,
                "message": "No text content found in PDF",
                "processing_time_ms": processing_time_ms,
                "embedding_time_ms": 0,
                "insert_time_ms": 0
            }

        # Generate embeddings for all chunks (with timing)
        texts = [chunk["text"] for chunk in chunks]
        t1 = time.perf_counter()
        embeddings = self.embedding_service.embed_batch(texts)
        embedding_time_ms = (time.perf_counter() - t1) * 1000

        # Prepare points for VelesDB
        points = []
        for i, (chunk, embedding) in enumerate(zip(chunks, embeddings)):
            # VelesDB expects u64 ID, convert hex string to int
            chunk_id = int(chunk["id"][:16], 16)  # Use first 16 hex chars (64 bits)
            points.append({
                "id": chunk_id,
                "vector": embedding,
                "payload": {
                    "chunk_id_hex": chunk["id"],  # Keep original for reference
                    "text": chunk["text"],
                    "document_name": chunk["document_name"],
                    "page_number": chunk["page_number"],
                    "chunk_index": chunk["chunk_index"]
                }
            })

        # Upsert to VelesDB (with timing)
        t2 = time.perf_counter()
        await self.velesdb.upsert_points(
            self.settings.collection_name,
            points
        )
        insert_time_ms = (time.perf_counter() - t2) * 1000

        # Track document metadata
        pages = set(c["page_number"] for c in chunks)
        self._documents[pdf_path.name] = {
            "name": pdf_path.name,
            "pages": len(pages),
            "chunks": len(chunks),
            "uploaded_at": datetime.now().isoformat()
        }

        return {
            "success": True,
            "document_name": pdf_path.name,
            "pages_processed": len(pages),
            "chunks_created": len(chunks),
            "message": f"Successfully indexed {len(chunks)} chunks from {len(pages)} pages",
            "processing_time_ms": round(processing_time_ms, 2),
            "embedding_time_ms": round(embedding_time_ms, 2),
            "insert_time_ms": round(insert_time_ms, 2)
        }

    async def search(
        self,
        query: str,
        top_k: int = 5,
        document_filter: str | None = None
    ) -> dict[str, Any]:
        """
        Search for relevant document chunks.

        Args:
            query: Search query text
            top_k: Number of results to return
            document_filter: Optional document name filter

        Returns:
            Search results with timing metrics
        """
        # Generate query embedding (with timing)
        t0 = time.perf_counter()
        query_embedding = self.embedding_service.embed(query)
        embedding_time_ms = (time.perf_counter() - t0) * 1000

        # Build filter if specified
        filter_ = None
        if document_filter:
            filter_ = {"document_name": {"eq": document_filter}}

        # Search VelesDB (with timing)
        t1 = time.perf_counter()
        results = await self.velesdb.search(
            collection=self.settings.collection_name,
            query_vector=query_embedding,
            top_k=top_k,
            filter_=filter_
        )
        search_time_ms = (time.perf_counter() - t1) * 1000

        # Format results
        formatted = []
        for result in results.get("results", []):
            payload = result.get("payload", {})
            formatted.append({
                "text": payload.get("text", ""),
                "document_name": payload.get("document_name", "unknown"),
                "page_number": payload.get("page_number", 0),
                "score": result.get("score", 0.0)
            })

        return {
            "results": formatted,
            "embedding_time_ms": round(embedding_time_ms, 2),
            "search_time_ms": round(search_time_ms, 2)
        }

    async def list_documents(self) -> list[dict[str, Any]]:
        """
        List all indexed documents.

        Returns:
            List of document metadata
        """
        return list(self._documents.values())

    async def delete_document(self, document_name: str) -> dict[str, Any]:
        """
        Delete a document and its chunks from the index.

        Args:
            document_name: Name of the document to delete

        Returns:
            Deletion result
        """
        result = await self.velesdb.delete_by_filter(
            collection=self.settings.collection_name,
            filter_={"document_name": {"eq": document_name}}
        )

        if document_name in self._documents:
            del self._documents[document_name]

        return result

    async def health_check(self) -> dict[str, Any]:
        """
        Check system health.

        Returns:
            Health status
        """
        velesdb_ok = False
        try:
            velesdb_ok = await self.velesdb.health_check()
        except Exception:
            pass

        return {
            "status": "healthy" if velesdb_ok else "degraded",
            "velesdb_connected": velesdb_ok,
            "embedding_model": self.settings.embedding_model,
            "embedding_dimension": self.settings.embedding_dimension,
            "documents_count": len(self._documents)
        }
