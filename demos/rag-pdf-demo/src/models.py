"""Pydantic models for API requests and responses."""

from pydantic import BaseModel, Field
from typing import Optional


class DocumentChunk(BaseModel):
    """A chunk of text from a document."""

    id: str
    text: str
    document_name: str
    page_number: int
    chunk_index: int


class SearchRequest(BaseModel):
    """Search query request."""

    query: str = Field(..., min_length=1, description="Search query text")
    top_k: int = Field(default=5, ge=1, le=100, description="Number of results")


class SearchResult(BaseModel):
    """A single search result."""

    text: str
    document_name: str
    page_number: int
    score: float


class SearchResponse(BaseModel):
    """Search response with results."""

    query: str
    results: list[SearchResult]
    total_results: int
    search_time_ms: float = Field(description="Search latency in milliseconds")
    embedding_time_ms: float = Field(description="Query embedding time in milliseconds")


class DocumentInfo(BaseModel):
    """Information about an uploaded document."""

    name: str
    pages: int
    chunks: int
    uploaded_at: str


class UploadResponse(BaseModel):
    """Response after document upload."""

    success: bool
    document_name: str
    pages_processed: int
    chunks_created: int
    message: str
    processing_time_ms: float = Field(default=0, description="PDF processing time in ms")
    embedding_time_ms: float = Field(default=0, description="Embedding generation time in ms")
    insert_time_ms: float = Field(default=0, description="VelesDB insert time in ms")


class HealthResponse(BaseModel):
    """Health check response."""

    status: str
    velesdb_connected: bool
    embedding_model: str
    embedding_dimension: int
    documents_count: int
