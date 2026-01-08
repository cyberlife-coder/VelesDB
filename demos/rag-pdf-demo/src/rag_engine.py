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
        
        # Load existing documents from VelesDB
        await self._load_existing_documents()

    async def _load_existing_documents(self) -> None:
        """Load document metadata from existing points in VelesDB."""
        try:
            # Use a dummy vector to search for all points with payload
            dummy_vector = [0.0] * self.embedding_service.dimension
            
            # Get collection info to know total points
            try:
                collection_info = await self.velesdb.get_collection_info(
                    self.settings.collection_name
                )
                total_points = collection_info.get("point_count", 0)
            except Exception:
                total_points = 0
            
            if total_points == 0:
                return  # No points to load
            
            # Search for ALL points to extract document names (no arbitrary limit)
            results = await self.velesdb.search(
                collection=self.settings.collection_name,
                query_vector=dummy_vector,
                top_k=total_points  # Get ALL chunks
            )
            
            # Extract unique document names from payloads
            documents_found = {}
            for result in results.get("results", []):
                payload = result.get("payload", {})
                doc_name = payload.get("document_name")
                if doc_name and doc_name not in documents_found:
                    documents_found[doc_name] = {
                        "name": doc_name,
                        "pages": 1,  # Will be updated below
                        "chunks": 0,
                        "chunk_ids": [],  # Track IDs for deletion
                        "uploaded_at": "existing"  # Existing document
                    }
            
            # Count chunks per document and collect chunk IDs
            for result in results.get("results", []):
                payload = result.get("payload", {})
                doc_name = payload.get("document_name")
                point_id = result.get("id")
                if doc_name in documents_found:
                    documents_found[doc_name]["chunks"] += 1
                    if point_id is not None:
                        documents_found[doc_name]["chunk_ids"].append(point_id)
                    # Track page numbers
                    page_num = payload.get("page_number", 0)
                    if "pages_set" not in documents_found[doc_name]:
                        documents_found[doc_name]["pages_set"] = set()
                    documents_found[doc_name]["pages_set"].add(page_num)
            
            # Update page counts
            for doc_name, doc_info in documents_found.items():
                if "pages_set" in doc_info:
                    doc_info["pages"] = len(doc_info["pages_set"])
                    del doc_info["pages_set"]  # Clean up temporary data
            
            # Update the documents registry
            self._documents.update(documents_found)
            
        except Exception as e:
            print(f"Warning: Could not load existing documents: {e}")
            # Continue with empty documents list

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
            # Use hash of full chunk ID for better collision resistance
            # chunk["id"] is 32 hex chars, take first 16 for u64 (still good distribution)
            chunk_id = int(chunk["id"][:16], 16)  # 64-bit ID from first half of MD5
            points.append({
                "id": chunk_id,
                "vector": embedding,
                "payload": {
                    "chunk_id_hex": chunk["id"],  # Keep full hash for reference
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

        # Track document metadata including chunk IDs for deletion
        pages = set(c["page_number"] for c in chunks)
        chunk_ids = [p["id"] for p in points]  # Store IDs for deletion
        self._documents[pdf_path.name] = {
            "name": pdf_path.name,
            "pages": len(pages),
            "chunks": len(chunks),
            "chunk_ids": chunk_ids,  # Track for deletion
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

    async def ingest_text(
        self,
        text: str,
        document_name: str
    ) -> dict[str, Any]:
        """
        Ingest raw text content into VelesDB.

        Args:
            text: Text content to ingest
            document_name: Name for the document

        Returns:
            Ingestion result with stats
        """
        await self.ensure_collection()

        # Chunk the text (with timing)
        t0 = time.perf_counter()
        chunks = self.pdf_processor.chunk_text(
            text=text,
            document_name=document_name,
            page_number=1,
            start_index=0
        )
        processing_time_ms = (time.perf_counter() - t0) * 1000

        if not chunks:
            return {
                "success": False,
                "document_name": document_name,
                "pages_processed": 0,
                "chunks_created": 0,
                "message": "No text content to index",
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
        for chunk, embedding in zip(chunks, embeddings):
            chunk_id = int(chunk["id"][:16], 16)
            points.append({
                "id": chunk_id,
                "vector": embedding,
                "payload": {
                    "chunk_id_hex": chunk["id"],
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

        # Track document metadata including chunk IDs for deletion
        chunk_ids = [p["id"] for p in points]
        self._documents[document_name] = {
            "name": document_name,
            "pages": 1,
            "chunks": len(chunks),
            "chunk_ids": chunk_ids,
            "uploaded_at": datetime.now().isoformat()
        }

        return {
            "success": True,
            "document_name": document_name,
            "pages_processed": 1,
            "chunks_created": len(chunks),
            "message": f"Successfully indexed {len(chunks)} chunks",
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
        Delete a document and all its chunks from VelesDB.

        Args:
            document_name: Name of the document to delete

        Returns:
            Deletion result with count of deleted chunks
        """
        if document_name not in self._documents:
            return {"deleted": 0, "message": "Document not found"}
        
        doc_info = self._documents[document_name]
        chunk_ids = doc_info.get("chunk_ids", [])
        deleted_count = 0
        errors = []
        
        # Delete each chunk from VelesDB
        for chunk_id in chunk_ids:
            try:
                await self.velesdb.delete_point(
                    self.settings.collection_name,
                    chunk_id
                )
                deleted_count += 1
            except Exception as e:
                # Log but continue deleting other chunks
                errors.append(f"Failed to delete chunk {chunk_id}: {e}")
        
        # Remove from local registry
        del self._documents[document_name]
        
        result = {
            "deleted": deleted_count,
            "message": f"Deleted {deleted_count}/{len(chunk_ids)} chunks"
        }
        
        if errors:
            result["errors"] = errors
        
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
