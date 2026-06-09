"""LangChain Memory integration for VelesDB AgentMemory (EPIC-010/US-006).

Provides LangChain-compatible memory classes backed by VelesDB:
- VelesDBChatMemory: Conversation history using EpisodicMemory
- VelesDBSemanticMemory: Fact storage for RAG using SemanticMemory

Example:
    >>> from langchain_velesdb import VelesDBChatMemory
    >>> from langchain.chains import ConversationChain
    >>> from langchain_openai import ChatOpenAI
    >>>
    >>> memory = VelesDBChatMemory(path="./agent_data")
    >>> chain = ConversationChain(llm=ChatOpenAI(), memory=memory)
    >>> response = chain.predict(input="Hello!")
"""

import json
from typing import Any, Dict, List, Optional, Union
import time

from langchain_velesdb._common import make_initial_id_counter
from velesdb_common.memory import (
    chronological,
    format_procedural_results,
    parse_event_entry,
    resolve_procedure_id,
    store_procedure,
)

try:
    from langchain.memory.chat_memory import BaseChatMemory
    from langchain.schema import (
        AIMessage,
        BaseMessage,
        ChatMessage,
        HumanMessage,
        SystemMessage,
    )
    from langchain_core.messages import ToolMessage
except ImportError:
    raise ImportError(
        "langchain is required for VelesDBChatMemory. "
        "Install with: pip install langchain"
    )

try:
    import velesdb
except ImportError:
    raise ImportError(
        "velesdb is required for VelesDBChatMemory. "
        "Install with: pip install velesdb"
    )


# Map a stored episodic role to its LangChain message class.  Roles that are
# not recognised are reconstructed as a generic ``ChatMessage`` so the original
# role string is preserved rather than silently collapsed to "human".
_ROLE_TO_MESSAGE = {
    "human": HumanMessage,
    "ai": AIMessage,
    "system": SystemMessage,
    "tool": ToolMessage,
}


def _event_to_message(role: str, content: str) -> BaseMessage:
    """Rebuild a LangChain message, preserving the original role."""
    message_cls = _ROLE_TO_MESSAGE.get(role)
    if message_cls is ToolMessage:
        return ToolMessage(content=content, tool_call_id="")
    if message_cls is not None:
        return message_cls(content=content)
    return ChatMessage(role=role, content=content)


class VelesDBChatMemory(BaseChatMemory):
    """LangChain chat memory backed by VelesDB EpisodicMemory.

    Stores conversation history as episodic events with timestamps,
    enabling temporal recall of recent messages.

    Args:
        path: Path to VelesDB database directory
        dimension: Embedding dimension (default: 384)
        window: Maximum number of past messages to load (default: 20)
        embedding: Optional LangChain Embeddings instance.  When provided,
            each recorded turn is embedded so that similarity recall over the
            conversation works; when omitted, turns are stored without vectors.
        memory_key: Key for memory variables (default: "history")
        human_prefix: Prefix for human messages (default: "Human")
        ai_prefix: Prefix for AI messages (default: "AI")
        return_messages: Return messages as objects vs string (default: False)

    Example:
        >>> memory = VelesDBChatMemory(path="./chat_data")
        >>> memory.save_context({"input": "Hi"}, {"output": "Hello!"})
        >>> memory.load_memory_variables({})
        {'history': 'Human: Hi\\nAI: Hello!'}
    """

    path: str
    dimension: int = 384
    window: int = 20
    memory_key: str = "history"
    human_prefix: str = "Human"
    ai_prefix: str = "AI"
    return_messages: bool = False

    _db: Any = None
    _memory: Any = None
    _embedding: Any = None
    _message_counter: int = 0
    _recorded_ids: List[int] = []
    _last_ts: int = 0

    class Config:
        arbitrary_types_allowed = True
        # Allow the underscore-prefixed runtime handles (`_db`, `_memory`, ...)
        # to be assigned in __init__. Without this, langchain's pydantic-v1
        # BaseChatMemory rejects assignment to names absent from `__fields__`.
        extra = "allow"

    def __init__(
        self,
        path: str,
        dimension: int = 384,
        window: int = 20,
        embedding: Optional[Any] = None,
        **kwargs,
    ):
        super().__init__(path=path, dimension=dimension, window=window, **kwargs)
        self._db = velesdb.Database(path)
        self._memory = self._db.agent_memory(dimension=dimension)
        self._embedding = embedding
        self._message_counter = make_initial_id_counter()
        self._recorded_ids = []
        self._last_ts = 0

    @property
    def memory_variables(self) -> List[str]:
        """Return memory variables."""
        return [self.memory_key]

    def load_memory_variables(self, inputs: Dict[str, Any]) -> Dict[str, Any]:
        """Load conversation history from VelesDB.

        Args:
            inputs: Input variables (unused but required by interface)

        Returns:
            Dict with memory_key containing conversation history
        """
        # Episodic ``recent`` returns newest-first; read oldest-first so a
        # turn's human prompt precedes its AI reply.
        events = chronological(self._memory.episodic.recent(limit=self.window))

        if self.return_messages:
            return {self.memory_key: self._events_to_messages(events)}
        return {self.memory_key: self._events_to_string(events)}

    def save_context(self, inputs: Dict[str, Any], outputs: Dict[str, str]) -> None:
        """Save conversation turn to VelesDB.

        Args:
            inputs: Input dict with user message
            outputs: Output dict with AI response
        """
        input_str = inputs.get("input", inputs.get("human_input", ""))
        output_str = outputs.get("output", outputs.get("response", ""))

        # Each message gets a strictly increasing timestamp so chronological
        # ordering is unambiguous even when several turns land in the same
        # wall-clock second (timestamp-bucket collisions would otherwise
        # interleave a later turn's prompt with an earlier turn's reply).
        self._record("human", input_str)
        self._record("ai", output_str)

    def _next_timestamp(self) -> int:
        """Return a strictly increasing millisecond timestamp."""
        self._last_ts = max(int(time.time() * 1000), self._last_ts + 1)
        return self._last_ts

    def _record(self, role: str, content: str) -> None:
        """Record one episodic turn, embedding it when a model is configured."""
        self._message_counter += 1
        embedding = self._embedding.embed_query(content) if self._embedding else None
        self._memory.episodic.record(
            event_id=self._message_counter,
            description=json.dumps({"role": role, "content": content}),
            timestamp=self._next_timestamp(),
            embedding=embedding,
        )
        self._recorded_ids.append(self._message_counter)

    def clear(self) -> None:
        """Delete this session's stored messages and reset state.

        Removes every episodic event recorded by this instance, clears the
        LangChain ``chat_memory`` buffer, and reseeds the ID counter so a
        fresh conversation starts with no prior context.
        """
        for event_id in self._recorded_ids:
            self._memory.episodic.delete(event_id)
        self._recorded_ids = []
        self._last_ts = 0
        self._message_counter = make_initial_id_counter()
        super().clear()

    def _events_to_messages(self, events: List) -> List[BaseMessage]:
        """Convert chronological episodic events to LangChain messages."""
        messages = []
        for event in events:
            role, content = parse_event_entry(event["description"])
            messages.append(_event_to_message(role, content))
        return messages

    def _events_to_string(self, events: List) -> str:
        """Convert chronological episodic events to a formatted string."""
        lines = []
        for event in events:
            role, content = parse_event_entry(event["description"])
            prefix = self.human_prefix if role == "human" else self.ai_prefix
            lines.append(f"{prefix}: {content}")
        return "\n".join(lines)


class VelesDBSemanticMemory:
    """Semantic memory for RAG using VelesDB SemanticMemory.

    Stores and retrieves facts with vector similarity search,
    ideal for building knowledge bases for RAG pipelines.

    Args:
        path: Path to VelesDB database directory
        dimension: Embedding dimension (must match your embeddings)
        embedding: LangChain Embeddings instance for encoding

    Example:
        >>> from langchain_openai import OpenAIEmbeddings
        >>> memory = VelesDBSemanticMemory(
        ...     path="./knowledge",
        ...     embedding=OpenAIEmbeddings()
        ... )
        >>> memory.add_fact("Paris is the capital of France")
        >>> facts = memory.query("What is the capital of France?", k=3)
    """

    def __init__(self, path: str, embedding: Any, dimension: Optional[int] = None):
        self.path = path
        self.embedding = embedding

        # Auto-detect dimension from embedding if not provided
        if dimension is None:
            sample = embedding.embed_query("test")
            dimension = len(sample)

        self.dimension = dimension
        self._db = velesdb.Database(path)
        self._memory = self._db.agent_memory(dimension=dimension)
        self._fact_counter = make_initial_id_counter()

    def add_fact(self, fact: str, fact_id: Optional[int] = None) -> int:
        """Add a fact to semantic memory.

        Args:
            fact: Text content of the fact
            fact_id: Optional custom ID (auto-generated if not provided)

        Returns:
            ID of the stored fact
        """
        if fact_id is None:
            self._fact_counter += 1
            fact_id = self._fact_counter

        # Generate embedding
        embedding = self.embedding.embed_query(fact)

        self._memory.semantic.store(fact_id, fact, embedding)
        return fact_id

    def add_facts(self, facts: List[str]) -> List[int]:
        """Add multiple facts to semantic memory.

        Args:
            facts: List of fact texts

        Returns:
            List of assigned fact IDs
        """
        ids = []
        for fact in facts:
            fact_id = self.add_fact(fact)
            ids.append(fact_id)
        return ids

    def query(self, query: str, k: int = 5) -> List[Dict[str, Any]]:
        """Query semantic memory for similar facts.

        Args:
            query: Query text
            k: Number of results to return

        Returns:
            List of dicts with 'id', 'content', 'score' keys
        """
        # Generate query embedding
        query_embedding = self.embedding.embed_query(query)

        # Search semantic memory
        results = self._memory.semantic.query(query_embedding, top_k=k)

        return results

    def clear(self) -> None:
        """Reset fact counter (facts persist in database)."""
        self._fact_counter = make_initial_id_counter()


class VelesDBProceduralMemory:
    """Procedural memory for AI agents using VelesDB.

    Stores learned procedures (named sequences of steps) with confidence
    scoring. Procedures can be recalled by embedding similarity and
    reinforced through success/failure feedback.

    Args:
        path: Path to VelesDB database directory.
        dimension: Embedding dimension (default: 384).
        embeddings: LangChain Embeddings instance used to encode the
            ``pattern`` string passed to :meth:`recall`.  Required for
            text-based recall; omit if you will always supply a raw
            embedding vector directly.

    Example:
        >>> from langchain_velesdb import VelesDBProceduralMemory
        >>> from langchain_openai import OpenAIEmbeddings
        >>> memory = VelesDBProceduralMemory(
        ...     path="./agent_data",
        ...     dimension=1536,
        ...     embeddings=OpenAIEmbeddings(),
        ... )
        >>> memory.learn("deploy_app", ["build", "test", "deploy"])
        >>> results = memory.recall("how to deploy")
        >>> memory.reinforce("deploy_app", success=True)
    """

    def __init__(
        self,
        path: str,
        dimension: int = 384,
        embeddings: Optional[Any] = None,
    ) -> None:
        self.path = path
        self._db = velesdb.Database(path)
        self._memory = self._db.agent_memory(dimension=dimension)
        self._procedural = self._memory.procedural
        self._embeddings = embeddings
        self._dimension = dimension
        # name → procedure_id mapping for reinforce() calls
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
            embedding: Optional vector representation.  When an
                ``embeddings`` model is configured and ``embedding`` is
                omitted, the name is embedded automatically.
            confidence: Initial confidence score in [0.0, 1.0].
        """
        emb = embedding
        if emb is None and self._embeddings is not None:
            emb = self._embeddings.embed_query(name)
        self._id_counter = store_procedure(
            self._procedural,
            name,
            steps,
            self._id_counter,
            self._name_to_id,
            emb,
            confidence,
        )

    def recall(
        self,
        pattern: str,
        top_k: int = 5,
        embedding: Optional[List[float]] = None,
        min_confidence: float = 0.0,
    ) -> List[Dict[str, Any]]:
        """Recall procedures matching the given pattern.

        Args:
            pattern: Text description used to generate a query embedding.
            top_k: Maximum number of results to return.
            embedding: Pre-computed query vector.  When provided, the
                ``pattern`` string is ignored for embedding generation.
            min_confidence: Minimum confidence threshold for results.

        Returns:
            List of dicts with ``id``, ``name``, ``steps``, ``confidence``,
            and ``score`` keys.

        Raises:
            RuntimeError: If no embeddings model is configured and no
                pre-computed ``embedding`` is provided.
        """
        if top_k < 1:
            raise ValueError(f"top_k must be >= 1, got {top_k}")

        query_emb = embedding
        if query_emb is None:
            if self._embeddings is None:
                raise RuntimeError(
                    "An embeddings model is required for text-based recall. "
                    "Pass embeddings= to VelesDBProceduralMemory() or supply "
                    "a pre-computed embedding= vector."
                )
            query_emb = self._embeddings.embed_query(pattern)

        results = self._procedural.recall(
            query_emb,
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
        self._procedural.reinforce(proc_id, success)

    def clear(self) -> None:
        """Reset the in-session procedure registry.

        Resets the name→ID mapping so previously learned names are no
        longer tracked for reinforcement.  The underlying VelesDB data
        is not deleted.
        """
        self._name_to_id = {}
        self._id_counter = make_initial_id_counter()
        self._memory = self._db.agent_memory(dimension=self._dimension)
        self._procedural = self._memory.procedural
