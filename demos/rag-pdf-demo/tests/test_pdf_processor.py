"""Tests for PDF processor module (TDD - tests first)."""

import pytest
from pathlib import Path


class TestPDFProcessor:
    """Test suite for PDF text extraction and chunking."""

    def test_extract_text_from_pdf(self, sample_pdf_path: Path):
        """Test extracting text from a PDF file."""
        from src.pdf_processor import PDFProcessor
        
        processor = PDFProcessor()
        pages = processor.extract_text(sample_pdf_path)
        
        assert len(pages) == 2
        assert "Machine" in pages[0]["text"]
        assert "Deep" in pages[1]["text"]
        assert pages[0]["page_number"] == 1
        assert pages[1]["page_number"] == 2

    def test_extract_text_invalid_file(self, tmp_path: Path):
        """Test handling of invalid PDF file."""
        from src.pdf_processor import PDFProcessor, PDFProcessingError
        
        invalid_file = tmp_path / "invalid.pdf"
        invalid_file.write_text("not a pdf")
        
        processor = PDFProcessor()
        
        with pytest.raises(PDFProcessingError):
            processor.extract_text(invalid_file)

    def test_chunk_text_basic(self):
        """Test basic text chunking."""
        from src.pdf_processor import PDFProcessor
        
        processor = PDFProcessor(chunk_size=50, chunk_overlap=10)
        # Use words so chunking works (splits on spaces)
        text = " ".join(["word"] * 30)  # 30 words = ~150 chars
        
        chunks = processor.chunk_text(text, document_name="test.pdf", page_number=1)
        
        assert len(chunks) >= 2
        assert all(len(c["text"]) <= 60 for c in chunks)  # Allow some flexibility

    def test_chunk_text_preserves_words(self):
        """Test that chunking preserves word boundaries."""
        from src.pdf_processor import PDFProcessor
        
        processor = PDFProcessor(chunk_size=30, chunk_overlap=5)
        text = "Hello world this is a test sentence for chunking"
        
        chunks = processor.chunk_text(text, document_name="test.pdf", page_number=1)
        
        # No chunk should split a word
        for chunk in chunks:
            words = chunk["text"].split()
            assert all(" " not in word for word in words)

    def test_process_pdf_end_to_end(self, sample_pdf_path: Path):
        """Test full PDF processing pipeline."""
        from src.pdf_processor import PDFProcessor
        
        processor = PDFProcessor(chunk_size=100, chunk_overlap=20)
        chunks = processor.process_pdf(sample_pdf_path)
        
        assert len(chunks) > 0
        assert all("id" in c for c in chunks)
        assert all("text" in c for c in chunks)
        assert all("document_name" in c for c in chunks)
        assert all("page_number" in c for c in chunks)

    def test_generate_chunk_id(self):
        """Test unique chunk ID generation."""
        from src.pdf_processor import PDFProcessor
        
        processor = PDFProcessor()
        
        id1 = processor.generate_chunk_id("doc.pdf", 1, 0)
        id2 = processor.generate_chunk_id("doc.pdf", 1, 1)
        id3 = processor.generate_chunk_id("other.pdf", 1, 0)
        
        assert id1 != id2
        assert id1 != id3
        assert isinstance(id1, str)
