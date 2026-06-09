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
from typing import Any, Dict, List, Optional, Union

import velesdb

from llamaindex_velesdb._common import make_initial_id_counter
from velesdb_common.memory import (
    chronological,
    format_procedural_results,
    parse_event_entry,
    resolve_procedure_id,
    store_procedure,
)

logger = logging.getLogger(__name__)


class _VelesDBMemoryBase:
    """Internal base providing common VelesDB database + memory initialisation.

    Shared by all three LlamaIndex memory classes to avoid repeating the
    ``db_path`` validation and ``agent_memory`` setup in every ``__init__``.
    """

    def __init__(self, db_path: str, dimension: int = 384) -> None:
        if not db_path:
            raise ValueError("db_path must not be empty")
        self._db = velesdb.Database(db_path)
        self._dimension = dimension
        self._memory = self._db.agent_memory(dimension=dimension)


class VelesDBSemanticMemory(_VelesDBMemoryBase):
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


class VelesDBEpisodicMemory(_VelesDBMemoryBase):
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
        super().__init__(db_path, dimension)
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

    def recent(self, limit: int = 20) -> List[Dict[str, Any]]:
        """Return the most recent events in chronological (oldest-first) order.

        VelesDB returns recent events newest-first; this method reverses
        them so the result reads as a forward timeline — the natural order
        for replaying a sequence of events or a conversation.

        Args:
            limit: Maximum number of events to return.

        Returns:
            List of dicts with ``id``, ``description``, and ``timestamp``
            keys, ordered oldest-first.

        Raises:
            ValueError: If ``limit`` is less than 1.
        """
        if limit < 1:
            raise ValueError(f"limit must be >= 1, got {limit}")
        return chronological(self._memory.episodic.recent(limit=limit))

    def clear(self) -> None:
        """Reinitialize the AgentMemory handle and reset the event counter.

        The underlying VelesDB collection is not deleted; only the
        in-process state is reset.
        """
        self._event_counter = make_initial_id_counter()
        self._memory = self._db.agent_memory(dimension=self._dimension)


def _role_label(role: str, human_prefix: str, ai_prefix: str) -> str:
    """Map a stored event role to its display prefix."""
    return human_prefix if role == "human" else ai_prefix


class VelesDBChatMemory(_VelesDBMemoryBase):
    """Chat-history memory backed by VelesDB episodic storage for LlamaIndex.

    Parity counterpart to ``langchain_velesdb.VelesDBChatMemory``: each
    conversation turn is stored as two timestamped episodic events
    (human + AI) and replayed in chronological order.

    Messages are recorded WITHOUT embeddings — conversation turns are
    retrieved by recency, not by vector similarity, so no embedding model
    is required.

    Args:
        db_path: Path to VelesDB database directory.
        dimension: Embedding dimension of the underlying store (default: 384).
        human_prefix: Prefix used for human turns in the string view.
        ai_prefix: Prefix used for AI turns in the string view.

    Example:
        >>> memory = VelesDBChatMemory(db_path="./chat")
        >>> memory.save_context({"input": "Hi"}, {"output": "Hello!"})
        >>> memory.load_memory_variables({})
        {'history': 'Human: Hi\\nAI: Hello!'}
    """

    def __init__(
        self,
        db_path: str,
        dimension: int = 384,
        human_prefix: str = "Human",
        ai_prefix: str = "AI",
    ) -> None:
        super().__init__(db_path, dimension)
        self._human_prefix = human_prefix
        self._ai_prefix = ai_prefix
        self._message_counter = make_initial_id_counter()
        self._last_timestamp = 0

    def _next_timestamp(self) -> int:
        """Return a strictly increasing per-message timestamp.

        Multiple messages recorded within the same wall-clock second would
        otherwise share a timestamp bucket, and episodic ``recent()`` orders
        within a bucket by descending ID — which scrambles turn order once
        reversed to chronological.  Forcing the timestamp to advance by at
        least one each call keeps every message in its own bucket so the
        timeline is unambiguous.
        """
        self._last_timestamp = max(int(time.time()), self._last_timestamp + 1)
        return self._last_timestamp

    def _record_turn(self, role: str, content: str) -> None:
        """Record one chat message as an episodic event."""
        self._message_counter += 1
        description = json.dumps({"role": role, "content": content})
        self._memory.episodic.record(
            self._message_counter, description, self._next_timestamp()
        )

    def save_context(
        self,
        inputs: Dict[str, Any],
        outputs: Dict[str, Any],
    ) -> None:
        """Persist one human/AI conversation turn.

        Args:
            inputs: Dict carrying the user message under ``"input"`` (or
                ``"human_input"``).
            outputs: Dict carrying the AI reply under ``"output"`` (or
                ``"response"``).
        """
        human = inputs.get("input", inputs.get("human_input", ""))
        ai = outputs.get("output", outputs.get("response", ""))
        self._record_turn("human", human)
        self._record_turn("ai", ai)

    def load_history(self, limit: int = 20) -> List[Any]:
        """Return chat history as LlamaIndex ``ChatMessage`` objects.

        Args:
            limit: Maximum number of messages to return (oldest-first).

        Returns:
            Chronologically ordered list of ``ChatMessage`` objects.
        """
        events = chronological(self._memory.episodic.recent(limit=limit))
        return [self._event_to_message(e) for e in events]

    def load_memory_variables(self, inputs: Dict[str, Any]) -> Dict[str, str]:
        """Return chat history as a single formatted string.

        Mirrors the LangChain adapter's default (string) view.

        Args:
            inputs: Unused; accepted for interface parity.

        Returns:
            ``{"history": "<prefix>: <content>\\n..."}`` oldest-first.
        """
        events = chronological(self._memory.episodic.recent(limit=20))
        lines = [self._event_to_line(e) for e in events]
        return {"history": "\n".join(lines)}

    def _event_to_line(self, event: Dict[str, Any]) -> str:
        """Format one stored event as a ``Prefix: content`` line."""
        role, content = parse_event_entry(event["description"])
        return f"{_role_label(role, self._human_prefix, self._ai_prefix)}: {content}"

    def _event_to_message(self, event: Dict[str, Any]) -> Any:
        """Convert one stored event into a LlamaIndex ``ChatMessage``."""
        from llama_index.core.base.llms.types import ChatMessage, MessageRole

        role, content = parse_event_entry(event["description"])
        msg_role = MessageRole.USER if role == "human" else MessageRole.ASSISTANT
        return ChatMessage(role=msg_role, content=content)

    def clear(self) -> None:
        """Reset the in-process message counter.

        Only the in-process ID counter and timestamp clock are reset;
        stored messages in the VelesDB episodic collection are NOT deleted.
        """
        self._message_counter = make_initial_id_counter()
        self._last_timestamp = 0


class VelesDBProceduralMemory(_VelesDBMemoryBase):
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
        super().__init__(db_path, dimension)
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
        self._id_counter = store_procedure(
            self._memory.procedural,
            name,
            steps,
            self._id_counter,
            self._name_to_id,
            embedding,
            confidence,
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
            List of dicts with ``id``, ``name``, ``steps``, ``confidence``,
            and ``score`` keys.  The ``id`` can be passed to
            :meth:`reinforce` to reinforce a recalled procedure that was
            not learned in the current session.

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

    def reinforce(self, name_or_id: Union[str, int], success: bool = True) -> None:
        """Reinforce or weaken a stored procedure.

        Accepts either a procedure ``name`` learned in the current session
        or a numeric ``id`` taken from a :meth:`recall` result.  The numeric
        form is what lets you reinforce procedures across sessions, where the
        in-memory name→ID registry is empty.

        Args:
            name_or_id: Name (``str``) or numeric ``id`` (``int``) of the
                procedure to update.
            success: ``True`` increases confidence; ``False`` decreases it.

        Raises:
            KeyError: If a ``name`` was not learned in this session.
        """
        proc_id = resolve_procedure_id(name_or_id, self._name_to_id)
        self._memory.procedural.reinforce(proc_id, success)

    def clear(self) -> None:
        """Reset the in-session procedure registry.

        Clears the name→ID mapping so previously learned names are no
        longer tracked for reinforcement.  The underlying VelesDB data
        is not deleted.
        """
        self._name_to_id = {}
        self._id_counter = make_initial_id_counter()
        self._memory = self._db.agent_memory(dimension=self._dimension)
