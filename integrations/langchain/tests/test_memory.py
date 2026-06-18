"""Tests for VelesDB LangChain Memory integration (EPIC-010/US-006)."""

import pytest
import tempfile

# Skip all tests if dependencies are not installed
pytest.importorskip("velesdb")
pytest.importorskip("langchain")


class TestVelesDBChatMemory:
    """Tests for VelesDBChatMemory."""

    def test_chat_memory_import(self):
        """Test: VelesDBChatMemory can be imported."""
        from langchain_velesdb import VelesDBChatMemory

        assert VelesDBChatMemory is not None  # guards __init__.py's `= None` ImportError fallback
        assert callable(VelesDBChatMemory)
        assert hasattr(VelesDBChatMemory, "save_context")
        assert hasattr(VelesDBChatMemory, "load_memory_variables")

    def test_chat_memory_initialization(self):
        """Test: VelesDBChatMemory can be initialized."""
        from langchain_velesdb import VelesDBChatMemory

        with tempfile.TemporaryDirectory() as tmpdir:
            memory = VelesDBChatMemory(path=tmpdir, dimension=4)
            assert memory is not None
            assert memory.path == tmpdir
            assert memory.dimension == 4
            assert memory._db is not None
            assert memory._memory is not None

    def test_chat_memory_save_and_load(self):
        """Test: VelesDBChatMemory can save and load context."""
        from langchain_velesdb import VelesDBChatMemory

        with tempfile.TemporaryDirectory() as tmpdir:
            memory = VelesDBChatMemory(path=tmpdir, dimension=4)

            # Save a conversation turn
            memory.save_context(
                {"input": "Hello, how are you?"},
                {"output": "I'm doing well, thank you!"},
            )

            # Load memory variables
            variables = memory.load_memory_variables({})

            assert "history" in variables
            assert "Hello" in variables["history"]
            assert "well" in variables["history"]

    def test_chat_memory_multiple_turns(self):
        """Test: VelesDBChatMemory handles multiple conversation turns."""
        from langchain_velesdb import VelesDBChatMemory

        with tempfile.TemporaryDirectory() as tmpdir:
            memory = VelesDBChatMemory(path=tmpdir, dimension=4)

            # Save multiple turns
            memory.save_context({"input": "Hi"}, {"output": "Hello!"})
            memory.save_context(
                {"input": "What's the weather?"}, {"output": "It's sunny today."}
            )

            variables = memory.load_memory_variables({})

            assert "Hi" in variables["history"]
            assert "Hello!" in variables["history"]
            assert "weather" in variables["history"]
            assert "sunny" in variables["history"]

    def test_chat_memory_return_messages(self):
        """Test: VelesDBChatMemory can return message objects."""
        from langchain_velesdb import VelesDBChatMemory
        from langchain.schema import HumanMessage, AIMessage

        with tempfile.TemporaryDirectory() as tmpdir:
            memory = VelesDBChatMemory(
                path=tmpdir, dimension=4, return_messages=True
            )

            memory.save_context({"input": "Test input"}, {"output": "Test output"})

            variables = memory.load_memory_variables({})
            messages = variables["history"]

            assert isinstance(messages, list)
            assert len(messages) >= 2

            # Check message types
            human_msgs = [m for m in messages if isinstance(m, HumanMessage)]
            ai_msgs = [m for m in messages if isinstance(m, AIMessage)]

            assert len(human_msgs) >= 1
            assert len(ai_msgs) >= 1

    def test_chat_memory_chronological_order(self):
        """Test: history is oldest-first across two turns (human before AI)."""
        from langchain_velesdb import VelesDBChatMemory

        with tempfile.TemporaryDirectory() as tmpdir:
            memory = VelesDBChatMemory(path=tmpdir, dimension=4)

            memory.save_context({"input": "turn one"}, {"output": "reply one"})
            memory.save_context({"input": "turn two"}, {"output": "reply two"})

            history = memory.load_memory_variables({})["history"]
            lines = history.split("\n")

            # Exactly the four messages, oldest first, each prompt before reply.
            assert lines == [
                "Human: turn one",
                "AI: reply one",
                "Human: turn two",
                "AI: reply two",
            ]

    def test_chat_memory_return_messages_order(self):
        """Test: message objects are oldest-first with correct types."""
        from langchain_velesdb import VelesDBChatMemory
        from langchain.schema import HumanMessage, AIMessage

        with tempfile.TemporaryDirectory() as tmpdir:
            memory = VelesDBChatMemory(
                path=tmpdir, dimension=4, return_messages=True
            )

            memory.save_context({"input": "q1"}, {"output": "a1"})
            memory.save_context({"input": "q2"}, {"output": "a2"})

            messages = memory.load_memory_variables({})["history"]
            types = [type(m) for m in messages]
            contents = [m.content for m in messages]

            assert types == [HumanMessage, AIMessage, HumanMessage, AIMessage]
            assert contents == ["q1", "a1", "q2", "a2"]

    def test_chat_memory_clear_empties_history(self):
        """Test: clear() deletes stored messages and reseeds the counter."""
        from langchain_velesdb import VelesDBChatMemory

        with tempfile.TemporaryDirectory() as tmpdir:
            memory = VelesDBChatMemory(path=tmpdir, dimension=4)

            memory.save_context({"input": "Hi"}, {"output": "Hello!"})
            assert memory.load_memory_variables({})["history"] != ""

            initial_counter = memory._message_counter
            memory.clear()

            # History is now empty and the counter was reseeded.
            assert memory.load_memory_variables({})["history"] == ""
            assert memory._recorded_ids == []
            assert memory._message_counter != initial_counter

    def test_chat_memory_records_embeddings_for_recall(self):
        """Test: when an embedding model is configured, turns are recallable."""
        from langchain_velesdb import VelesDBChatMemory

        class MockEmbedding:
            def embed_query(self, text: str):
                # "weather" turns embed onto one axis, everything else onto another.
                if "weather" in text or "sunny" in text:
                    return [1.0, 0.0, 0.0, 0.0]
                return [0.0, 1.0, 0.0, 0.0]

        with tempfile.TemporaryDirectory() as tmpdir:
            memory = VelesDBChatMemory(
                path=tmpdir, dimension=4, embedding=MockEmbedding()
            )

            memory.save_context({"input": "hello there"}, {"output": "hi"})
            memory.save_context(
                {"input": "what is the weather?"}, {"output": "it is sunny"}
            )

            results = memory._memory.episodic.recall_similar(
                [1.0, 0.0, 0.0, 0.0], top_k=2
            )
            # Embeddings were stored, so similarity recall returns the
            # weather turn (impossible if turns were recorded without vectors).
            assert len(results) >= 1
            contents = " ".join(r["description"] for r in results)
            assert "weather" in contents or "sunny" in contents

    def test_chat_memory_preserves_system_role(self):
        """Test: a stored 'system' role is reconstructed as SystemMessage."""
        from langchain_velesdb import VelesDBChatMemory
        from langchain.schema import SystemMessage

        with tempfile.TemporaryDirectory() as tmpdir:
            memory = VelesDBChatMemory(
                path=tmpdir, dimension=4, return_messages=True
            )
            # Record a system message directly via the internal recorder.
            memory._record("system", "you are a helpful assistant")

            messages = memory.load_memory_variables({})["history"]
            assert len(messages) == 1
            assert isinstance(messages[0], SystemMessage)
            assert messages[0].content == "you are a helpful assistant"


class TestVelesDBSemanticMemory:
    """Tests for VelesDBSemanticMemory."""

    def test_semantic_memory_import(self):
        """Test: VelesDBSemanticMemory can be imported."""
        import inspect
        from langchain_velesdb import VelesDBSemanticMemory

        # On ImportError the package binds this name to None (see __init__.py
        # optional-import block); assert the real class loaded instead.
        assert inspect.isclass(VelesDBSemanticMemory)
        assert VelesDBSemanticMemory.__name__ == "VelesDBSemanticMemory"

    def test_semantic_memory_initialization(self):
        """Test: VelesDBSemanticMemory can be initialized with mock embedding."""
        from langchain_velesdb import VelesDBSemanticMemory

        class MockEmbedding:
            def embed_query(self, text: str):
                return [0.1, 0.2, 0.3, 0.4]

        with tempfile.TemporaryDirectory() as tmpdir:
            memory = VelesDBSemanticMemory(
                path=tmpdir, embedding=MockEmbedding(), dimension=4
            )
            assert memory is not None
            assert memory.dimension == 4
            assert memory._db is not None
            assert memory._memory is not None

    def test_semantic_memory_add_fact(self):
        """Test: VelesDBSemanticMemory can add facts."""
        from langchain_velesdb import VelesDBSemanticMemory

        class MockEmbedding:
            def embed_query(self, text: str):
                return [0.1, 0.2, 0.3, 0.4]

        with tempfile.TemporaryDirectory() as tmpdir:
            memory = VelesDBSemanticMemory(
                path=tmpdir, embedding=MockEmbedding(), dimension=4
            )

            fact_id = memory.add_fact("Paris is the capital of France")
            assert fact_id > 0

    def test_semantic_memory_query(self):
        """Test: VelesDBSemanticMemory can query facts."""
        from langchain_velesdb import VelesDBSemanticMemory

        class MockEmbedding:
            def embed_query(self, text: str):
                # Return slightly different embeddings based on content
                if "Paris" in text or "capital" in text:
                    return [1.0, 0.0, 0.0, 0.0]
                return [0.5, 0.5, 0.0, 0.0]

        with tempfile.TemporaryDirectory() as tmpdir:
            memory = VelesDBSemanticMemory(
                path=tmpdir, embedding=MockEmbedding(), dimension=4
            )

            # Add a fact
            memory.add_fact("Paris is the capital of France")

            # Query
            results = memory.query("What is the capital of France?", k=1)

            assert len(results) >= 1

    def test_semantic_memory_add_facts_batch(self):
        """Test: VelesDBSemanticMemory can add multiple facts."""
        from langchain_velesdb import VelesDBSemanticMemory

        class MockEmbedding:
            def embed_query(self, text: str):
                return [0.1, 0.2, 0.3, 0.4]

        with tempfile.TemporaryDirectory() as tmpdir:
            memory = VelesDBSemanticMemory(
                path=tmpdir, embedding=MockEmbedding(), dimension=4
            )

            facts = [
                "The sky is blue",
                "Water is wet",
                "Fire is hot",
            ]

            ids = memory.add_facts(facts)

            assert len(ids) == 3
            assert all(id > 0 for id in ids)


class TestVelesDBProceduralMemoryClear:
    """Tests for procedural memory clear() ID collision fix."""

    def test_learn_after_clear_produces_different_id(self):
        """learn() after clear() must produce a different ID than learn() before clear()."""
        from langchain_velesdb.memory import VelesDBProceduralMemory

        with tempfile.TemporaryDirectory() as tmpdir:
            memory = VelesDBProceduralMemory(path=tmpdir, dimension=4)

            # Learn a procedure before clear
            memory.learn("greet", ["say hello", "wave"])
            id_before = memory._name_to_id.get("greet")

            # Clear and learn again with the same name
            memory.clear()
            memory.learn("greet", ["say hello", "wave"])
            id_after = memory._name_to_id.get("greet")

            assert id_before is not None
            assert id_after is not None
            assert id_before != id_after, (
                f"ID collision: learn() before and after clear() produced the same ID {id_before}"
            )
