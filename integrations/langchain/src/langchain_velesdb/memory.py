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
from typing import Any, Dict, List, Optional
import time

from langchain_velesdb._common import make_initial_id_counter, parse_event_entry
from velesdb_common.memory import format_procedural_results

try:
    from langchain.memory.chat_memory import BaseChatMemory
    from langchain.schema import BaseMessage, HumanMessage, AIMessage
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


class VelesDBChatMemory(BaseChatMemory):
    """LangChain chat memory backed by VelesDB EpisodicMemory.

    Stores conversation history as episodic events with timestamps,
    enabling temporal recall of recent messages.

    Args:
        path: Path to VelesDB database directory
        dimension: Embedding dimension (default: 384)
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
    memory_key: str = "history"
    human_prefix: str = "Human"
    ai_prefix: str = "AI"
    return_messages: bool = False

    _db: Any = None
    _memory: Any = None
    _message_counter: int = 0

    class Config:
        arbitrary_types_allowed = True

    def __init__(self, path: str, dimension: int = 384, **kwargs):
        super().__init__(**kwargs)
        self.path = path
        self.dimension = dimension
        self._db = velesdb.Database(path)
        self._memory = self._db.agent_memory(dimension=dimension)
        self._message_counter = make_initial_id_counter()

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
        # Get recent events from episodic memory
        recent_events = self._memory.episodic.recent(limit=20)

        if self.return_messages:
            messages = self._events_to_messages(recent_events)
            return {self.memory_key: messages}
        else:
            history_str = self._events_to_string(recent_events)
            return {self.memory_key: history_str}

    def save_context(self, inputs: Dict[str, Any], outputs: Dict[str, str]) -> None:
        """Save conversation turn to VelesDB.

        Args:
            inputs: Input dict with user message
            outputs: Output dict with AI response
        """
        input_str = inputs.get("input", inputs.get("human_input", ""))
        output_str = outputs.get("output", outputs.get("response", ""))

        timestamp = int(time.time())

        # Save human message
        self._message_counter += 1
        self._memory.episodic.record(
            event_id=self._message_counter,
            description=json.dumps({"role": "human", "content": input_str}),
            timestamp=timestamp,
        )

        # Save AI message
        self._message_counter += 1
        self._memory.episodic.record(
            event_id=self._message_counter,
            description=json.dumps({"role": "ai", "content": output_str}),
            timestamp=timestamp + 1,  # Slightly after human message
        )

    def clear(self) -> None:
        """Reset the message counter for this session.

        Note: This only resets the in-process counter used for ID
        generation.  Stored messages in the VelesDB episodic collection
        are NOT deleted.  To start a fresh conversation without old
        context, open a new database path or collection.
        """
        self._message_counter = make_initial_id_counter()

    def _events_to_messages(self, events: List) -> List[BaseMessage]:
        """Convert episodic events to LangChain messages."""
        messages = []
        for _event_id, description, _timestamp in events:
            role, content = parse_event_entry(description)
            if role == "human":
                messages.append(HumanMessage(content=content))
            else:
                messages.append(AIMessage(content=content))
        return messages

    def _events_to_string(self, events: List) -> str:
        """Convert episodic events to formatted string."""
        lines = []
        for _event_id, description, _timestamp in events:
            role, content = parse_event_entry(description)
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
        db_path: Path to VelesDB database directory.
        dimension: Embedding dimension (default: 384).
        embeddings: LangChain Embeddings instance used to encode the
            ``pattern`` string passed to :meth:`recall`.  Required for
            text-based recall; omit if you will always supply a raw
            embedding vector directly.

    Example:
        >>> from langchain_velesdb import VelesDBProceduralMemory
        >>> from langchain_openai import OpenAIEmbeddings
        >>> memory = VelesDBProceduralMemory(
        ...     db_path="./agent_data",
        ...     dimension=1536,
        ...     embeddings=OpenAIEmbeddings(),
        ... )
        >>> memory.learn("deploy_app", ["build", "test", "deploy"])
        >>> results = memory.recall("how to deploy")
        >>> memory.reinforce("deploy_app", success=True)
    """

    def __init__(
        self,
        db_path: str,
        dimension: int = 384,
        embeddings: Optional[Any] = None,
    ) -> None:
        self._db = velesdb.Database(db_path)
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
        if not name:
            raise ValueError("Procedure name must not be empty")
        if not steps:
            raise ValueError("Procedure steps must not be empty")

        emb = embedding
        if emb is None and self._embeddings is not None:
            emb = self._embeddings.embed_query(name)

        self._id_counter += 1
        proc_id = self._id_counter
        self._name_to_id[name] = proc_id
        self._procedural.learn(
            proc_id,
            name,
            steps,
            embedding=emb,
            confidence=confidence,
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
            List of dicts with ``name``, ``steps``, ``confidence``, and
            ``score`` keys.

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

    def reinforce(self, name: str, success: bool = True) -> None:
        """Reinforce or weaken a stored procedure.

        Args:
            name: Name of the procedure to update.
            success: ``True`` increases confidence; ``False`` decreases it.

        Raises:
            KeyError: If the procedure name has not been learned in this
                session.
        """
        if name not in self._name_to_id:
            raise KeyError(
                f"Unknown procedure '{name}'. "
                "Call learn() before reinforce()."
            )
        self._procedural.reinforce(self._name_to_id[name], success)

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
