"""PDF text extraction and chunking."""

import hashlib
from pathlib import Path
from typing import TypedDict

import fitz  # PyMuPDF

from .config import get_settings


class PageContent(TypedDict):
    """Content extracted from a PDF page."""

    text: str
    page_number: int


class ChunkData(TypedDict):
    """A chunk of document text."""

    id: str
    text: str
    document_name: str
    page_number: int
    chunk_index: int


class PDFProcessingError(Exception):
    """Error during PDF processing."""

    pass


class PDFProcessor:
    """Extract text from PDFs and chunk it for embedding."""

    def __init__(
        self,
        chunk_size: int | None = None,
        chunk_overlap: int | None = None
    ):
        settings = get_settings()
        self.chunk_size = chunk_size or settings.chunk_size
        self.chunk_overlap = chunk_overlap or settings.chunk_overlap

    def extract_text(self, pdf_path: Path) -> list[PageContent]:
        """
        Extract text from each page of a PDF.

        Args:
            pdf_path: Path to the PDF file

        Returns:
            List of page contents with text and page number

        Raises:
            PDFProcessingError: If the file cannot be processed
        """
        try:
            doc = fitz.open(pdf_path)
        except Exception as e:
            raise PDFProcessingError(f"Failed to open PDF: {e}") from e

        pages: list[PageContent] = []

        try:
            for page_num, page in enumerate(doc, start=1):
                text = page.get_text()
                if text.strip():
                    pages.append({
                        "text": text.strip(),
                        "page_number": page_num
                    })
        except Exception as e:
            raise PDFProcessingError(f"Failed to extract text: {e}") from e
        finally:
            doc.close()

        return pages

    def chunk_text(
        self,
        text: str,
        document_name: str,
        page_number: int,
        start_index: int = 0
    ) -> list[ChunkData]:
        """
        Split text into overlapping chunks preserving word boundaries.

        Args:
            text: Text to chunk
            document_name: Name of the source document
            page_number: Page number in the document
            start_index: Starting chunk index

        Returns:
            List of chunk data dictionaries
        """
        if not text.strip():
            return []

        words = text.split()
        chunks: list[ChunkData] = []
        current_chunk: list[str] = []
        current_length = 0
        chunk_index = start_index

        for word in words:
            word_len = len(word) + 1  # +1 for space

            if current_length + word_len > self.chunk_size and current_chunk:
                # Save current chunk
                chunk_text = " ".join(current_chunk)
                chunks.append({
                    "id": self.generate_chunk_id(document_name, page_number, chunk_index),
                    "text": chunk_text,
                    "document_name": document_name,
                    "page_number": page_number,
                    "chunk_index": chunk_index
                })
                chunk_index += 1

                # Calculate overlap
                overlap_words = []
                overlap_length = 0
                for w in reversed(current_chunk):
                    if overlap_length + len(w) + 1 <= self.chunk_overlap:
                        overlap_words.insert(0, w)
                        overlap_length += len(w) + 1
                    else:
                        break

                current_chunk = overlap_words
                current_length = overlap_length

            current_chunk.append(word)
            current_length += word_len

        # Don't forget the last chunk
        if current_chunk:
            chunk_text = " ".join(current_chunk)
            chunks.append({
                "id": self.generate_chunk_id(document_name, page_number, chunk_index),
                "text": chunk_text,
                "document_name": document_name,
                "page_number": page_number,
                "chunk_index": chunk_index
            })

        return chunks

    def generate_chunk_id(
        self,
        document_name: str,
        page_number: int,
        chunk_index: int
    ) -> str:
        """
        Generate a unique ID for a chunk.

        Args:
            document_name: Name of the document
            page_number: Page number
            chunk_index: Index of the chunk

        Returns:
            Unique string ID
        """
        content = f"{document_name}:{page_number}:{chunk_index}"
        return hashlib.md5(content.encode()).hexdigest()[:16]

    def process_pdf(self, pdf_path: Path) -> list[ChunkData]:
        """
        Process a PDF file: extract text and create chunks.

        Args:
            pdf_path: Path to the PDF file

        Returns:
            List of all chunks from the document
        """
        document_name = pdf_path.name
        pages = self.extract_text(pdf_path)

        all_chunks: list[ChunkData] = []
        global_chunk_index = 0

        for page in pages:
            chunks = self.chunk_text(
                text=page["text"],
                document_name=document_name,
                page_number=page["page_number"],
                start_index=global_chunk_index
            )
            all_chunks.extend(chunks)
            global_chunk_index += len(chunks)

        return all_chunks
