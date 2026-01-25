"""Tests for VelesDB LangChain Memory integration (EPIC-010/US-006)."""

import pytest
import tempfile
import os
import json

# Skip all tests if dependencies are not installed
pytest.importorskip("velesdb")
pytest.importorskip("langchain")


class TestVelesDBChatMemory:
    """Tests for VelesDBChatMemory."""

    def test_chat_memory_import(self):
        """Test: VelesDBChatMemory can be imported."""
        from langchain_velesdb import VelesDBChatMemory

        assert VelesDBChatMemory is not None

    def test_chat_memory_initialization(self):
        """Test: VelesDBChatMemory can be initialized."""
        from langchain_velesdb import VelesDBChatMemory

        with tempfile.TemporaryDirectory() as tmpdir:
            memory = VelesDBChatMemory(path=tmpdir, dimension=4)
            assert memory is not None
            assert memory.path == tmpdir
            assert memory.dimension == 4

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

    def test_chat_memory_clear(self):
        """Test: VelesDBChatMemory clear resets counter."""
        from langchain_velesdb import VelesDBChatMemory

        with tempfile.TemporaryDirectory() as tmpdir:
            memory = VelesDBChatMemory(path=tmpdir, dimension=4)

            initial_counter = memory._message_counter
            memory.clear()

            # Counter should be reset to new timestamp
            assert memory._message_counter != initial_counter


class TestVelesDBSemanticMemory:
    """Tests for VelesDBSemanticMemory."""

    def test_semantic_memory_import(self):
        """Test: VelesDBSemanticMemory can be imported."""
        from langchain_velesdb import VelesDBSemanticMemory

        assert VelesDBSemanticMemory is not None

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
