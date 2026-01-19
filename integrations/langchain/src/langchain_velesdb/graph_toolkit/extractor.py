"""Entity and relation extraction from unstructured text.

Supports local LLM extraction via Ollama for privacy-first workflows.
"""

from dataclasses import dataclass, field
from typing import Optional, List, Dict, Any, Protocol
import json
import re


@dataclass
class Entity:
    """Represents an extracted entity (node in the knowledge graph)."""

    name: str
    entity_type: str
    properties: Dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> Dict[str, Any]:
        """Convert to dictionary for serialization."""
        return {
            "name": self.name,
            "type": self.entity_type,
            "properties": self.properties,
        }


@dataclass
class Relation:
    """Represents a relation between entities (edge in the knowledge graph)."""

    source: str
    target: str
    relation_type: str
    properties: Dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> Dict[str, Any]:
        """Convert to dictionary for serialization."""
        return {
            "source": self.source,
            "target": self.target,
            "type": self.relation_type,
            "properties": self.properties,
        }


@dataclass
class ExtractionResult:
    """Result of entity/relation extraction."""

    entities: List[Entity] = field(default_factory=list)
    relations: List[Relation] = field(default_factory=list)
    source_text: str = ""
    metadata: Dict[str, Any] = field(default_factory=dict)

    @property
    def nodes(self) -> List[Entity]:
        """Alias for entities."""
        return self.entities

    @property
    def edges(self) -> List[Relation]:
        """Alias for relations."""
        return self.relations


class LLMClient(Protocol):
    """Protocol for LLM client interface."""

    def generate(self, prompt: str) -> str:
        """Generate text from prompt."""
        ...


class OllamaClient:
    """Client for Ollama local LLM."""

    def __init__(self, model: str = "llama3", base_url: str = "http://localhost:11434"):
        self.model = model
        self.base_url = base_url

    def generate(self, prompt: str) -> str:
        """Generate text using Ollama API."""
        try:
            import requests
        except ImportError:
            raise ImportError("requests package required: pip install requests")

        response = requests.post(
            f"{self.base_url}/api/generate",
            json={"model": self.model, "prompt": prompt, "stream": False},
            timeout=120,
        )
        response.raise_for_status()
        return response.json().get("response", "")


EXTRACTION_PROMPT = """Extract entities and relationships from the following text.

TEXT:
{text}

Output a JSON object with this exact structure:
{{
  "entities": [
    {{"name": "entity name", "type": "PERSON|ORGANIZATION|LOCATION|CONCEPT|EVENT|OTHER", "properties": {{}}}}
  ],
  "relations": [
    {{"source": "entity1 name", "target": "entity2 name", "type": "WORKS_AT|LOCATED_IN|KNOWS|PART_OF|RELATED_TO|OTHER", "properties": {{}}}}
  ]
}}

Be thorough but only extract entities and relations that are clearly stated or strongly implied.
Output ONLY valid JSON, no explanations."""


class GraphExtractor:
    """Extracts entities and relations from text using LLMs.

    Supports local LLMs via Ollama for privacy-first extraction.

    Args:
        model: Model name for Ollama (default: "llama3").
        ollama_url: Ollama API URL (default: "http://localhost:11434").
        llm_client: Optional custom LLM client implementing LLMClient protocol.

    Example:
        >>> extractor = GraphExtractor(model="llama3")
        >>> result = extractor.extract("John works at Acme Corp.")
        >>> print(result.entities)
        [Entity(name='John', type='PERSON'), Entity(name='Acme Corp', type='ORGANIZATION')]
    """

    def __init__(
        self,
        model: str = "llama3",
        ollama_url: str = "http://localhost:11434",
        llm_client: Optional[LLMClient] = None,
    ):
        self.model = model
        self.ollama_url = ollama_url
        self._client = llm_client

    @property
    def client(self) -> LLMClient:
        """Get or create LLM client."""
        if self._client is None:
            self._client = OllamaClient(model=self.model, base_url=self.ollama_url)
        return self._client

    def extract(self, text: str) -> ExtractionResult:
        """Extract entities and relations from text.

        Args:
            text: Input text to analyze.

        Returns:
            ExtractionResult with entities and relations.
        """
        if not text.strip():
            return ExtractionResult(source_text=text)

        prompt = EXTRACTION_PROMPT.format(text=text)
        response = self.client.generate(prompt)

        return self._parse_response(response, text)

    def extract_batch(self, texts: List[str]) -> List[ExtractionResult]:
        """Extract entities and relations from multiple texts.

        Args:
            texts: List of input texts.

        Returns:
            List of ExtractionResult objects.
        """
        return [self.extract(text) for text in texts]

    def _parse_response(self, response: str, source_text: str) -> ExtractionResult:
        """Parse LLM response into ExtractionResult."""
        try:
            json_match = re.search(r"\{.*\}", response, re.DOTALL)
            if not json_match:
                return ExtractionResult(source_text=source_text)

            data = json.loads(json_match.group())

            entities = [
                Entity(
                    name=e.get("name", ""),
                    entity_type=e.get("type", "OTHER"),
                    properties=e.get("properties", {}),
                )
                for e in data.get("entities", [])
                if e.get("name")
            ]

            relations = [
                Relation(
                    source=r.get("source", ""),
                    target=r.get("target", ""),
                    relation_type=r.get("type", "RELATED_TO"),
                    properties=r.get("properties", {}),
                )
                for r in data.get("relations", [])
                if r.get("source") and r.get("target")
            ]

            return ExtractionResult(
                entities=entities,
                relations=relations,
                source_text=source_text,
            )

        except (json.JSONDecodeError, KeyError):
            return ExtractionResult(source_text=source_text)
