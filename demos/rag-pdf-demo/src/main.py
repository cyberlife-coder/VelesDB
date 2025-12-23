"""FastAPI application for RAG demo."""

import tempfile
from contextlib import asynccontextmanager
from pathlib import Path

from fastapi import FastAPI, File, HTTPException, UploadFile
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import FileResponse, HTMLResponse
from fastapi.staticfiles import StaticFiles

from .models import (
    HealthResponse,
    SearchRequest,
    SearchResponse,
    SearchResult,
    UploadResponse,
)
from .rag_engine import RAGEngine

# Global RAG engine instance
rag_engine: RAGEngine | None = None


@asynccontextmanager
async def lifespan(app: FastAPI):
    """Application lifespan handler."""
    global rag_engine
    rag_engine = RAGEngine()
    
    # Ensure collection exists on startup
    try:
        await rag_engine.ensure_collection()
    except Exception as e:
        print(f"Warning: Could not connect to VelesDB: {e}")
    
    yield
    
    # Cleanup
    rag_engine = None


app = FastAPI(
    title="VelesDB RAG Demo",
    description="PDF Question Answering with VelesDB vector search",
    version="0.1.0",
    lifespan=lifespan
)

# CORS for frontend
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

# Static files
static_path = Path(__file__).parent.parent / "static"
if static_path.exists():
    app.mount("/static", StaticFiles(directory=str(static_path)), name="static")


@app.get("/", response_class=HTMLResponse)
async def root():
    """Serve the main page."""
    index_file = static_path / "index.html"
    if index_file.exists():
        return FileResponse(index_file)
    return HTMLResponse(
        content="""
        <html>
            <head><title>VelesDB RAG Demo</title></head>
            <body>
                <h1>VelesDB RAG Demo</h1>
                <p>API is running. Use /docs for Swagger UI.</p>
            </body>
        </html>
        """
    )


@app.get("/health", response_model=HealthResponse)
async def health_check():
    """Check API and VelesDB health."""
    if rag_engine is None:
        raise HTTPException(status_code=503, detail="Service not ready")
    
    health = await rag_engine.health_check()
    return HealthResponse(**health)


@app.post("/documents/upload", response_model=UploadResponse)
async def upload_document(file: UploadFile = File(...)):
    """
    Upload and index a PDF document.
    
    The document will be:
    1. Parsed to extract text
    2. Split into chunks
    3. Embedded using sentence-transformers
    4. Stored in VelesDB for fast retrieval
    """
    if rag_engine is None:
        raise HTTPException(status_code=503, detail="Service not ready")
    
    if not file.filename or not file.filename.lower().endswith(".pdf"):
        raise HTTPException(
            status_code=400,
            detail="Only PDF files are supported"
        )
    
    # Save uploaded file temporarily
    with tempfile.NamedTemporaryFile(suffix=".pdf", delete=False) as tmp:
        content = await file.read()
        tmp.write(content)
        tmp_path = Path(tmp.name)
    
    try:
        result = await rag_engine.ingest_document(tmp_path)
        
        return UploadResponse(
            success=result["success"],
            document_name=file.filename,
            pages_processed=result["pages_processed"],
            chunks_created=result["chunks_created"],
            message=result["message"],
            processing_time_ms=result.get("processing_time_ms", 0),
            embedding_time_ms=result.get("embedding_time_ms", 0),
            insert_time_ms=result.get("insert_time_ms", 0)
        )
    except Exception as e:
        import traceback
        print(f"ERROR uploading document: {e}")
        print(traceback.format_exc())
        raise HTTPException(status_code=500, detail=str(e))
    finally:
        # Cleanup temp file
        tmp_path.unlink(missing_ok=True)


@app.post("/search", response_model=SearchResponse)
async def search_documents(request: SearchRequest):
    """
    Search indexed documents using semantic similarity.
    
    The query will be:
    1. Embedded using the same model as documents
    2. Searched in VelesDB using cosine similarity
    3. Top-k results returned with scores
    """
    if rag_engine is None:
        raise HTTPException(status_code=503, detail="Service not ready")
    
    try:
        result = await rag_engine.search(
            query=request.query,
            top_k=request.top_k
        )
        
        return SearchResponse(
            query=request.query,
            results=[SearchResult(**r) for r in result["results"]],
            total_results=len(result["results"]),
            search_time_ms=result["search_time_ms"],
            embedding_time_ms=result["embedding_time_ms"]
        )
    except Exception as e:
        raise HTTPException(status_code=500, detail=str(e))


@app.get("/documents")
async def list_documents():
    """List all indexed documents."""
    if rag_engine is None:
        raise HTTPException(status_code=503, detail="Service not ready")
    
    documents = await rag_engine.list_documents()
    return {"documents": documents}


@app.delete("/documents/{document_name}")
async def delete_document(document_name: str):
    """Delete a document from the index."""
    if rag_engine is None:
        raise HTTPException(status_code=503, detail="Service not ready")
    
    try:
        result = await rag_engine.delete_document(document_name)
        return {"success": True, "deleted": result.get("deleted", 0)}
    except Exception as e:
        raise HTTPException(status_code=500, detail=str(e))


if __name__ == "__main__":
    import uvicorn
    uvicorn.run(app, host="0.0.0.0", port=8000)
