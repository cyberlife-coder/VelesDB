"""VelesDB Agent Memory integration for LlamaIndex.

Provides semantic, episodic, and procedural memory for AI agent workflows.

Each class wraps one of the three VelesDB AgentMemory subsystems and
exposes a lightweight, LlamaIndex-friendly API:

- :class:`VelesDBSemanticMemory`  — long-term knowledge facts
- :class:`VelesDBEpisodicMemory`  — timestamped event timeline
- :class:`VelesDBProceduralMemory` — learned step sequences
"""

from __future__ import annotations

import json
import logging
import time
from typing import Any, Dict, List, Optional

import velesdb

from llamaindex_velesdb._common import make_initial_id_counter
from velesdb_common.memory import format_procedural_results

logger = logging.getLogger(__name__)


class VelesDBSemanticMemory:
    """Semantic memory backed by VelesDB for LlamaIndex agent workflows.

    Stores named knowledge facts with embedding vectors and retrieves them
    by vector similarity.

    Args:
        db_path: Path to VelesDB database directory.
        dimension: Embedding dimension (default: 384).

    Example:
        >>> memory = VelesDBSemanticMemory(db_path="./data", dimension=768)
        >>> memory.add_fact(1, "Paris is the capital of France", embedding)
        >>> results = memory.query(query_embedding, top_k=3)
    """

    def __init__(self, db_path: str, dimension: int = 384) -> None:
        if not db_path:
            raise ValueError("db_path must not be empty")
        self._db = velesdb.Database(db_path)
        self._dimension = dimension
        self._memory = self._db.agent_memory(dimension=dimension)

    def add_fact(
        self,
        fact_id: int,
        text: str,
        embedding: List[float],
        metadata: Optional[Dict[str, Any]] = None,
    ) -> None:
        """Store a knowledge fact with its embedding.

        Args:
            fact_id: Unique numeric identifier for the fact.
            text: Text content of the knowledge.
            embedding: Vector representation matching the configured dimension.
            metadata: Unused; reserved for future payload support.

        Raises:
            ValueError: If ``text`` is empty or ``embedding`` is empty.
        """
        if not text:
            raise ValueError("text must not be empty")
        if not embedding:
            raise ValueError("embedding must not be empty")
        self._memory.semantic.store(fact_id, text, embedding)

    def query(
        self,
        embedding: List[float],
        top_k: int = 5,
    ) -> List[Dict[str, Any]]:
        """Retrieve facts by vector similarity.

        Args:
            embedding: Query vector matching the configured dimension.
            top_k: Maximum number of results to return.

        Returns:
            List of dicts with ``id``, ``content``, and ``score`` keys.

        Raises:
            ValueError: If ``top_k`` is less than 1 or ``embedding`` is empty.
        """
        if top_k < 1:
            raise ValueError(f"top_k must be >= 1, got {top_k}")
        if not embedding:
            raise ValueError("embedding must not be empty")
        return self._memory.semantic.query(embedding, top_k=top_k)

    def clear(self) -> None:
        """Reinitialize the AgentMemory handle.

        The underlying VelesDB collection is not deleted; only the
        in-process memory handle is reset.
        """
        self._memory = self._db.agent_memory(dimension=self._dimension)


class VelesDBEpisodicMemory:
    """Episodic memory backed by VelesDB for LlamaIndex agent workflows.

    Records timestamped events and retrieves them by recency or embedding
    similarity.

    Args:
        db_path: Path to VelesDB database directory.
        dimension: Embedding dimension (default: 384).

    Example:
        >>> memory = VelesDBEpisodicMemory(db_path="./data")
        >>> memory.record_event("user_message", {"text": "Hello"}, embedding)
        >>> recent = memory.recall(query_embedding, top_k=5)
    """

    def __init__(self, db_path: str, dimension: int = 384) -> None:
        if not db_path:
            raise ValueError("db_path must not be empty")
        self._db = velesdb.Database(db_path)
        self._dimension = dimension
        self._memory = self._db.agent_memory(dimension=dimension)
        self._event_counter = make_initial_id_counter()

    def record_event(
        self,
        event_type: str,
        data: Dict[str, Any],
        embedding: List[float],
        metadata: Optional[Dict[str, Any]] = None,
    ) -> int:
        """Record a new event in episodic memory.

        Args:
            event_type: Category label for the event (e.g. ``"user_message"``).
            data: Arbitrary data payload serialised into the description.
            embedding: Vector representation for similarity recall.
            metadata: Unused; reserved for future payload support.

        Returns:
            Numeric event ID assigned to the stored event.

        Raises:
            ValueError: If ``event_type`` is empty or ``embedding`` is empty.
        """
        if not event_type:
            raise ValueError("event_type must not be empty")
        if not embedding:
            raise ValueError("embedding must not be empty")

        self._event_counter += 1
        event_id = self._event_counter
        description = json.dumps({"type": event_type, "data": data})
        timestamp = int(time.time())
        self._memory.episodic.record(
            event_id,
            description,
            timestamp,
            embedding=embedding,
        )
        return event_id

    def recall(
        self,
        embedding: List[float],
        top_k: int = 5,
    ) -> List[Dict[str, Any]]:
        """Recall events similar to the given embedding.

        Args:
            embedding: Query vector.
            top_k: Maximum number of results to return.

        Returns:
            List of dicts with ``id``, ``description``, ``timestamp``,
            and ``score`` keys.

        Raises:
            ValueError: If ``top_k`` is less than 1 or ``embedding`` is empty.
        """
        if top_k < 1:
            raise ValueError(f"top_k must be >= 1, got {top_k}")
        if not embedding:
            raise ValueError("embedding must not be empty")
        return self._memory.episodic.recall_similar(embedding, top_k=top_k)

    def clear(self) -> None:
        """Reinitialize the AgentMemory handle and reset the event counter.

        The underlying VelesDB collection is not deleted; only the
        in-process state is reset.
        """
        self._event_counter = make_initial_id_counter()
        self._memory = self._db.agent_memory(dimension=self._dimension)


class VelesDBProceduralMemory:
    """Procedural memory backed by VelesDB for LlamaIndex agent workflows.

    Stores named procedures (ordered step sequences) with confidence
    scoring.  Procedures are recalled by embedding similarity and can be
    reinforced through success/failure feedback.

    Args:
        db_path: Path to VelesDB database directory.
        dimension: Embedding dimension (default: 384).

    Example:
        >>> memory = VelesDBProceduralMemory(db_path="./data", dimension=384)
        >>> memory.learn("deploy_app", ["build", "test", "deploy"],
        ...              embedding=my_embedding)
        >>> results = memory.recall(query_embedding, top_k=3)
        >>> memory.reinforce("deploy_app", success=True)
    """

    def __init__(self, db_path: str, dimension: int = 384) -> None:
        if not db_path:
            raise ValueError("db_path must not be empty")
        self._db = velesdb.Database(db_path)
        self._dimension = dimension
        self._memory = self._db.agent_memory(dimension=dimension)
        self._name_to_id: Dict[str, int] = {}
        self._id_counter = make_initial_id_counter()

    def learn(
        self,
        name: str,
        steps: List[str],
        metadata: Optional[Dict[str, Any]] = None,
        embedding: Optional[List[float]] = None,
        confidence: float = 0.5,
    ) -> None:
        """Store a procedure under the given name.

        Args:
            name: Human-readable identifier for the procedure.
            steps: Ordered list of action steps.
            metadata: Unused; reserved for future payload support.
            embedding: Optional vector representation for similarity recall.
            confidence: Initial confidence in [0.0, 1.0] (default: 0.5).

        Raises:
            ValueError: If ``name`` or ``steps`` is empty.
        """
        if not name:
            raise ValueError("Procedure name must not be empty")
        if not steps:
            raise ValueError("Procedure steps must not be empty")

        self._id_counter += 1
        proc_id = self._id_counter
        self._name_to_id[name] = proc_id
        self._memory.procedural.learn(
            proc_id,
            name,
            steps,
            embedding=embedding,
            confidence=confidence,
        )

    def recall(
        self,
        embedding: List[float],
        top_k: int = 5,
        min_confidence: float = 0.0,
    ) -> List[Dict[str, Any]]:
        """Recall procedures similar to the given embedding.

        Args:
            embedding: Query vector.
            top_k: Maximum number of results to return.
            min_confidence: Minimum confidence threshold for results.

        Returns:
            List of dicts with ``name``, ``steps``, ``confidence``,
            and ``score`` keys.

        Raises:
            ValueError: If ``top_k`` is less than 1 or ``embedding`` is empty.
        """
        if top_k < 1:
            raise ValueError(f"top_k must be >= 1, got {top_k}")
        if not embedding:
            raise ValueError("embedding must not be empty")

        results = self._memory.procedural.recall(
            embedding,
            top_k=top_k,
            min_confidence=min_confidence,
        )
        return format_procedural_results(results)

    def reinforce(self, name: str, success: bool = True) -> None:
        """Reinforce or weaken a stored procedure.

        Args:
            name: Name of the procedure to update.
            success: ``True`` increases confidence; ``False`` decreases it.

        Raises:
            KeyError: If ``name`` was not learned in this session.
        """
        if name not in self._name_to_id:
            raise KeyError(
                f"Unknown procedure '{name}'. "
                "Call learn() before reinforce()."
            )
        self._memory.procedural.reinforce(self._name_to_id[name], success)

    def clear(self) -> None:
        """Reset the in-session procedure registry.

        Clears the name→ID mapping so previously learned names are no
        longer tracked for reinforcement.  The underlying VelesDB data
        is not deleted.
        """
        self._name_to_id = {}
        self._id_counter = make_initial_id_counter()
        self._memory = self._db.agent_memory(dimension=self._dimension)
