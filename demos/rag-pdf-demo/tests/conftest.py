"""Pytest fixtures for RAG demo tests."""

import pytest
from pathlib import Path
import tempfile


@pytest.fixture
def sample_pdf_path(tmp_path: Path) -> Path:
    """Create a sample PDF for testing."""
    import fitz  # PyMuPDF

    pdf_path = tmp_path / "test_document.pdf"
    doc = fitz.open()
    
    # Page 1
    page1 = doc.new_page()
    page1.insert_text(
        (50, 100),
        "Machine Learning Introduction\n\n"
        "Machine learning is a subset of artificial intelligence (AI) "
        "that provides systems the ability to automatically learn and "
        "improve from experience without being explicitly programmed.",
        fontsize=12
    )
    
    # Page 2
    page2 = doc.new_page()
    page2.insert_text(
        (50, 100),
        "Deep Learning\n\n"
        "Deep learning is part of a broader family of machine learning "
        "methods based on artificial neural networks with representation "
        "learning. Learning can be supervised, semi-supervised or unsupervised.",
        fontsize=12
    )
    
    doc.save(str(pdf_path))
    doc.close()
    
    return pdf_path


@pytest.fixture
def sample_text() -> str:
    """Sample text for embedding tests."""
    return "VelesDB is a high-performance vector database for AI applications."


@pytest.fixture
def sample_chunks() -> list[dict]:
    """Sample document chunks for testing."""
    return [
        {
            "id": "doc1_0",
            "text": "Machine learning is a subset of AI.",
            "document_name": "test.pdf",
            "page_number": 1,
            "chunk_index": 0
        },
        {
            "id": "doc1_1",
            "text": "Deep learning uses neural networks.",
            "document_name": "test.pdf",
            "page_number": 2,
            "chunk_index": 1
        }
    ]
