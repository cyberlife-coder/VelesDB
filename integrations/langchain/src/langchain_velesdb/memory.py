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

from typing import Any, Dict, List, Optional
import time
import json
import uuid

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
        # Use timestamp + UUID suffix to avoid collisions between concurrent instances
        self._message_counter = int(time.time() * 1000) + (uuid.uuid4().int % 1000000)

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
        """Clear conversation history.

        Note: This creates a new AgentMemory instance, effectively
        clearing the episodic memory for new conversations.
        """
        # Reinitialize memory (collections will be reused but new session)
        self._message_counter = int(time.time() * 1000) + (uuid.uuid4().int % 1000000)

    def _events_to_messages(self, events: List) -> List[BaseMessage]:
        """Convert episodic events to LangChain messages."""
        messages = []
        for event_id, description, timestamp in events:
            try:
                data = json.loads(description)
                role = data.get("role", "human")
                content = data.get("content", description)

                if role == "human":
                    messages.append(HumanMessage(content=content))
                else:
                    messages.append(AIMessage(content=content))
            except json.JSONDecodeError:
                # Fallback for non-JSON descriptions
                messages.append(HumanMessage(content=description))

        return messages

    def _events_to_string(self, events: List) -> str:
        """Convert episodic events to formatted string."""
        lines = []
        for event_id, description, timestamp in events:
            try:
                data = json.loads(description)
                role = data.get("role", "human")
                content = data.get("content", description)

                prefix = self.human_prefix if role == "human" else self.ai_prefix
                lines.append(f"{prefix}: {content}")
            except json.JSONDecodeError:
                lines.append(f"{self.human_prefix}: {description}")

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
        self._fact_counter = int(time.time() * 1000)

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
        self._fact_counter = int(time.time() * 1000)
