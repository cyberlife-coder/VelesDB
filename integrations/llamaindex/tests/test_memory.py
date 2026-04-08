"""Tests for VelesDB LlamaIndex Memory integration (EPIC-010/US-006).

Covers all three memory types (Semantic, Episodic, Procedural) with nominal,
edge-case, and error-path scenarios.  All tests run against an embedded VelesDB
instance in a temporary directory — no server required.

Run with: pytest tests/test_memory.py -v
"""

from __future__ import annotations

import pytest

pytest.importorskip("velesdb")

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

SMALL_DIM = 4
UNIT_EMBEDDING = [1.0, 0.0, 0.0, 0.0]
OTHER_EMBEDDING = [0.0, 1.0, 0.0, 0.0]


@pytest.fixture
def tmp_db(tmp_path):
    """Yield a fresh temporary directory path for each test."""
    return str(tmp_path)


# ---------------------------------------------------------------------------
# Semantic memory
# ---------------------------------------------------------------------------


class TestSemanticMemory:
    """Tests for VelesDBSemanticMemory."""

    def test_import(self):
        """VelesDBSemanticMemory can be imported."""
        from llamaindex_velesdb.memory import VelesDBSemanticMemory

        assert VelesDBSemanticMemory is not None

    def test_init(self, tmp_db):
        """Initialisation succeeds with a valid path and dimension."""
        from llamaindex_velesdb.memory import VelesDBSemanticMemory

        memory = VelesDBSemanticMemory(db_path=tmp_db, dimension=SMALL_DIM)
        assert memory is not None

    def test_empty_db_path_raises(self):
        """Empty db_path must raise ValueError at construction time."""
        from llamaindex_velesdb.memory import VelesDBSemanticMemory

        with pytest.raises(ValueError, match="db_path"):
            VelesDBSemanticMemory(db_path="", dimension=SMALL_DIM)

    def test_add_fact(self, tmp_db):
        """add_fact stores a knowledge fact without error."""
        from llamaindex_velesdb.memory import VelesDBSemanticMemory

        memory = VelesDBSemanticMemory(db_path=tmp_db, dimension=SMALL_DIM)
        # No return value to check; just assert no exception is raised.
        memory.add_fact(1, "Paris is the capital of France", UNIT_EMBEDDING)

    def test_add_fact_empty_text_raises(self, tmp_db):
        """add_fact with empty text must raise ValueError."""
        from llamaindex_velesdb.memory import VelesDBSemanticMemory

        memory = VelesDBSemanticMemory(db_path=tmp_db, dimension=SMALL_DIM)
        with pytest.raises(ValueError, match="text"):
            memory.add_fact(1, "", UNIT_EMBEDDING)

    def test_add_fact_empty_embedding_raises(self, tmp_db):
        """add_fact with empty embedding must raise ValueError."""
        from llamaindex_velesdb.memory import VelesDBSemanticMemory

        memory = VelesDBSemanticMemory(db_path=tmp_db, dimension=SMALL_DIM)
        with pytest.raises(ValueError, match="embedding"):
            memory.add_fact(1, "Some fact", [])

    def test_query_returns_list(self, tmp_db):
        """query returns a list after a fact is stored."""
        from llamaindex_velesdb.memory import VelesDBSemanticMemory

        memory = VelesDBSemanticMemory(db_path=tmp_db, dimension=SMALL_DIM)
        memory.add_fact(1, "Paris is the capital of France", UNIT_EMBEDDING)
        results = memory.query(UNIT_EMBEDDING, top_k=1)
        assert isinstance(results, list)
        assert len(results) >= 1

    def test_query_result_keys(self, tmp_db):
        """query results expose id, content, and score keys."""
        from llamaindex_velesdb.memory import VelesDBSemanticMemory

        memory = VelesDBSemanticMemory(db_path=tmp_db, dimension=SMALL_DIM)
        memory.add_fact(1, "Water is wet", UNIT_EMBEDDING)
        results = memory.query(UNIT_EMBEDDING, top_k=1)
        assert len(results) >= 1
        first = results[0]
        assert "id" in first
        assert "content" in first
        assert "score" in first

    def test_query_empty_collection_returns_empty(self, tmp_db):
        """query on an empty collection returns an empty list."""
        from llamaindex_velesdb.memory import VelesDBSemanticMemory

        memory = VelesDBSemanticMemory(db_path=tmp_db, dimension=SMALL_DIM)
        results = memory.query(UNIT_EMBEDDING, top_k=3)
        assert results == []

    def test_query_invalid_top_k_raises(self, tmp_db):
        """query with top_k < 1 must raise ValueError."""
        from llamaindex_velesdb.memory import VelesDBSemanticMemory

        memory = VelesDBSemanticMemory(db_path=tmp_db, dimension=SMALL_DIM)
        with pytest.raises(ValueError, match="top_k"):
            memory.query(UNIT_EMBEDDING, top_k=0)

    def test_query_empty_embedding_raises(self, tmp_db):
        """query with an empty embedding must raise ValueError."""
        from llamaindex_velesdb.memory import VelesDBSemanticMemory

        memory = VelesDBSemanticMemory(db_path=tmp_db, dimension=SMALL_DIM)
        with pytest.raises(ValueError, match="embedding"):
            memory.query([], top_k=1)

    def test_clear_resets_state(self, tmp_db):
        """clear reinitialises the memory handle without raising."""
        from llamaindex_velesdb.memory import VelesDBSemanticMemory

        memory = VelesDBSemanticMemory(db_path=tmp_db, dimension=SMALL_DIM)
        memory.add_fact(1, "Fact before clear", UNIT_EMBEDDING)
        memory.clear()
        # After clear the object must still be usable.
        memory.add_fact(2, "Fact after clear", UNIT_EMBEDDING)

    def test_query_after_clear_still_works(self, tmp_db):
        """query remains usable after clear."""
        from llamaindex_velesdb.memory import VelesDBSemanticMemory

        memory = VelesDBSemanticMemory(db_path=tmp_db, dimension=SMALL_DIM)
        memory.clear()
        # query on a freshly cleared handle must not raise.
        results = memory.query(UNIT_EMBEDDING, top_k=1)
        assert isinstance(results, list)


# ---------------------------------------------------------------------------
# Episodic memory
# ---------------------------------------------------------------------------


class TestEpisodicMemory:
    """Tests for VelesDBEpisodicMemory."""

    def test_import(self):
        """VelesDBEpisodicMemory can be imported."""
        from llamaindex_velesdb.memory import VelesDBEpisodicMemory

        assert VelesDBEpisodicMemory is not None

    def test_init(self, tmp_db):
        """Initialisation succeeds with a valid path and dimension."""
        from llamaindex_velesdb.memory import VelesDBEpisodicMemory

        memory = VelesDBEpisodicMemory(db_path=tmp_db, dimension=SMALL_DIM)
        assert memory is not None

    def test_record_event_returns_positive_id(self, tmp_db):
        """record_event returns a positive numeric event ID."""
        from llamaindex_velesdb.memory import VelesDBEpisodicMemory

        memory = VelesDBEpisodicMemory(db_path=tmp_db, dimension=SMALL_DIM)
        event_id = memory.record_event(
            "user_message", {"text": "Hello"}, UNIT_EMBEDDING
        )
        assert isinstance(event_id, int)
        assert event_id > 0

    def test_record_multiple_events_ids_are_monotonically_increasing(self, tmp_db):
        """Each successive record_event call must produce a larger ID."""
        from llamaindex_velesdb.memory import VelesDBEpisodicMemory

        memory = VelesDBEpisodicMemory(db_path=tmp_db, dimension=SMALL_DIM)
        id_a = memory.record_event("msg", {"text": "first"}, UNIT_EMBEDDING)
        id_b = memory.record_event("msg", {"text": "second"}, OTHER_EMBEDDING)
        assert id_b > id_a

    def test_record_event_empty_event_type_raises(self, tmp_db):
        """record_event with empty event_type must raise ValueError."""
        from llamaindex_velesdb.memory import VelesDBEpisodicMemory

        memory = VelesDBEpisodicMemory(db_path=tmp_db, dimension=SMALL_DIM)
        with pytest.raises(ValueError, match="event_type"):
            memory.record_event("", {"text": "Hello"}, UNIT_EMBEDDING)

    def test_record_event_empty_embedding_raises(self, tmp_db):
        """record_event with empty embedding must raise ValueError."""
        from llamaindex_velesdb.memory import VelesDBEpisodicMemory

        memory = VelesDBEpisodicMemory(db_path=tmp_db, dimension=SMALL_DIM)
        with pytest.raises(ValueError, match="embedding"):
            memory.record_event("msg", {"text": "Hi"}, [])

    def test_recall_returns_list(self, tmp_db):
        """recall returns a list after an event is stored."""
        from llamaindex_velesdb.memory import VelesDBEpisodicMemory

        memory = VelesDBEpisodicMemory(db_path=tmp_db, dimension=SMALL_DIM)
        memory.record_event("user_message", {"text": "Hello"}, UNIT_EMBEDDING)
        results = memory.recall(UNIT_EMBEDDING, top_k=1)
        assert isinstance(results, list)
        assert len(results) >= 1

    def test_recall_empty_collection_returns_empty(self, tmp_db):
        """recall on an empty episodic store returns an empty list."""
        from llamaindex_velesdb.memory import VelesDBEpisodicMemory

        memory = VelesDBEpisodicMemory(db_path=tmp_db, dimension=SMALL_DIM)
        results = memory.recall(UNIT_EMBEDDING, top_k=5)
        assert results == []

    def test_recall_invalid_top_k_raises(self, tmp_db):
        """recall with top_k < 1 must raise ValueError."""
        from llamaindex_velesdb.memory import VelesDBEpisodicMemory

        memory = VelesDBEpisodicMemory(db_path=tmp_db, dimension=SMALL_DIM)
        with pytest.raises(ValueError, match="top_k"):
            memory.recall(UNIT_EMBEDDING, top_k=0)

    def test_recall_negative_top_k_raises(self, tmp_db):
        """recall with negative top_k must raise ValueError."""
        from llamaindex_velesdb.memory import VelesDBEpisodicMemory

        memory = VelesDBEpisodicMemory(db_path=tmp_db, dimension=SMALL_DIM)
        with pytest.raises(ValueError, match="top_k"):
            memory.recall(UNIT_EMBEDDING, top_k=-1)

    def test_recall_empty_embedding_raises(self, tmp_db):
        """recall with empty embedding must raise ValueError."""
        from llamaindex_velesdb.memory import VelesDBEpisodicMemory

        memory = VelesDBEpisodicMemory(db_path=tmp_db, dimension=SMALL_DIM)
        with pytest.raises(ValueError, match="embedding"):
            memory.recall([], top_k=1)

    def test_clear_resets_event_counter(self, tmp_db):
        """clear resets the event counter so IDs don't collide with prior session."""
        from llamaindex_velesdb.memory import VelesDBEpisodicMemory

        memory = VelesDBEpisodicMemory(db_path=tmp_db, dimension=SMALL_DIM)
        id_before_clear = memory.record_event("msg", {}, UNIT_EMBEDDING)

        memory.clear()
        id_after_clear = memory.record_event("msg", {}, UNIT_EMBEDDING)

        assert id_after_clear != id_before_clear, (
            "ID collision: record_event() before and after clear() "
            f"produced the same ID {id_before_clear}"
        )

    def test_recall_after_clear_still_works(self, tmp_db):
        """recall is functional immediately after clear."""
        from llamaindex_velesdb.memory import VelesDBEpisodicMemory

        memory = VelesDBEpisodicMemory(db_path=tmp_db, dimension=SMALL_DIM)
        memory.clear()
        results = memory.recall(UNIT_EMBEDDING, top_k=1)
        assert isinstance(results, list)


# ---------------------------------------------------------------------------
# Procedural memory
# ---------------------------------------------------------------------------


class TestProceduralMemory:
    """Tests for VelesDBProceduralMemory."""

    def test_import(self):
        """VelesDBProceduralMemory can be imported."""
        from llamaindex_velesdb.memory import VelesDBProceduralMemory

        assert VelesDBProceduralMemory is not None

    def test_init(self, tmp_db):
        """Initialisation succeeds with a valid path and dimension."""
        from llamaindex_velesdb.memory import VelesDBProceduralMemory

        memory = VelesDBProceduralMemory(db_path=tmp_db, dimension=SMALL_DIM)
        assert memory is not None

    def test_learn_stores_procedure(self, tmp_db):
        """learn succeeds and the name is tracked in _name_to_id."""
        from llamaindex_velesdb.memory import VelesDBProceduralMemory

        memory = VelesDBProceduralMemory(db_path=tmp_db, dimension=SMALL_DIM)
        memory.learn("deploy_app", ["build", "test", "deploy"])
        assert "deploy_app" in memory._name_to_id

    def test_learn_assigns_positive_id(self, tmp_db):
        """The ID assigned to a learned procedure is positive."""
        from llamaindex_velesdb.memory import VelesDBProceduralMemory

        memory = VelesDBProceduralMemory(db_path=tmp_db, dimension=SMALL_DIM)
        memory.learn("greet", ["say hello", "wave"])
        proc_id = memory._name_to_id["greet"]
        assert proc_id > 0

    def test_learn_empty_name_raises(self, tmp_db):
        """learn with empty name must raise ValueError."""
        from llamaindex_velesdb.memory import VelesDBProceduralMemory

        memory = VelesDBProceduralMemory(db_path=tmp_db, dimension=SMALL_DIM)
        with pytest.raises(ValueError, match="name"):
            memory.learn("", ["step1"])

    def test_learn_empty_steps_raises(self, tmp_db):
        """learn with empty steps must raise ValueError."""
        from llamaindex_velesdb.memory import VelesDBProceduralMemory

        memory = VelesDBProceduralMemory(db_path=tmp_db, dimension=SMALL_DIM)
        with pytest.raises(ValueError, match="steps"):
            memory.learn("no_steps", [])

    def test_learn_with_embedding(self, tmp_db):
        """learn with an explicit embedding vector stores the procedure."""
        from llamaindex_velesdb.memory import VelesDBProceduralMemory

        memory = VelesDBProceduralMemory(db_path=tmp_db, dimension=SMALL_DIM)
        memory.learn(
            "deploy_app",
            ["build", "test", "deploy"],
            embedding=UNIT_EMBEDDING,
        )
        assert "deploy_app" in memory._name_to_id

    def test_recall_returns_list(self, tmp_db):
        """recall returns a list after a procedure is learned."""
        from llamaindex_velesdb.memory import VelesDBProceduralMemory

        memory = VelesDBProceduralMemory(db_path=tmp_db, dimension=SMALL_DIM)
        memory.learn("greet", ["say hello"], embedding=UNIT_EMBEDDING)
        results = memory.recall(UNIT_EMBEDDING, top_k=1)
        assert isinstance(results, list)

    def test_recall_result_keys(self, tmp_db):
        """recall results expose name, steps, confidence, and score keys."""
        from llamaindex_velesdb.memory import VelesDBProceduralMemory

        memory = VelesDBProceduralMemory(db_path=tmp_db, dimension=SMALL_DIM)
        memory.learn("greet", ["say hello"], embedding=UNIT_EMBEDDING)
        results = memory.recall(UNIT_EMBEDDING, top_k=1)
        assert len(results) >= 1
        first = results[0]
        assert "name" in first
        assert "steps" in first
        assert "confidence" in first
        assert "score" in first

    def test_recall_empty_collection_returns_empty(self, tmp_db):
        """recall on an empty procedural store returns an empty list."""
        from llamaindex_velesdb.memory import VelesDBProceduralMemory

        memory = VelesDBProceduralMemory(db_path=tmp_db, dimension=SMALL_DIM)
        results = memory.recall(UNIT_EMBEDDING, top_k=5)
        assert results == []

    def test_recall_invalid_top_k_raises(self, tmp_db):
        """recall with top_k < 1 must raise ValueError."""
        from llamaindex_velesdb.memory import VelesDBProceduralMemory

        memory = VelesDBProceduralMemory(db_path=tmp_db, dimension=SMALL_DIM)
        with pytest.raises(ValueError, match="top_k"):
            memory.recall(UNIT_EMBEDDING, top_k=0)

    def test_recall_negative_top_k_raises(self, tmp_db):
        """recall with negative top_k must raise ValueError."""
        from llamaindex_velesdb.memory import VelesDBProceduralMemory

        memory = VelesDBProceduralMemory(db_path=tmp_db, dimension=SMALL_DIM)
        with pytest.raises(ValueError, match="top_k"):
            memory.recall(UNIT_EMBEDDING, top_k=-3)

    def test_recall_empty_embedding_raises(self, tmp_db):
        """recall with empty embedding must raise ValueError."""
        from llamaindex_velesdb.memory import VelesDBProceduralMemory

        memory = VelesDBProceduralMemory(db_path=tmp_db, dimension=SMALL_DIM)
        with pytest.raises(ValueError, match="embedding"):
            memory.recall([], top_k=1)

    def test_reinforce_known_procedure(self, tmp_db):
        """reinforce does not raise for a previously learned procedure."""
        from llamaindex_velesdb.memory import VelesDBProceduralMemory

        memory = VelesDBProceduralMemory(db_path=tmp_db, dimension=SMALL_DIM)
        memory.learn("greet", ["say hello"], embedding=UNIT_EMBEDDING)
        memory.reinforce("greet", success=True)

    def test_reinforce_unknown_procedure_raises(self, tmp_db):
        """reinforce with an unknown name must raise KeyError."""
        from llamaindex_velesdb.memory import VelesDBProceduralMemory

        memory = VelesDBProceduralMemory(db_path=tmp_db, dimension=SMALL_DIM)
        with pytest.raises(KeyError, match="ghost"):
            memory.reinforce("ghost", success=True)

    def test_clear_resets_name_registry(self, tmp_db):
        """clear empties _name_to_id so the name is no longer tracked."""
        from llamaindex_velesdb.memory import VelesDBProceduralMemory

        memory = VelesDBProceduralMemory(db_path=tmp_db, dimension=SMALL_DIM)
        memory.learn("greet", ["say hello", "wave"])
        memory.clear()
        assert len(memory._name_to_id) == 0

    def test_learn_after_clear_produces_different_id(self, tmp_db):
        """learn() after clear() must produce a different ID than before clear()."""
        from llamaindex_velesdb.memory import VelesDBProceduralMemory

        memory = VelesDBProceduralMemory(db_path=tmp_db, dimension=SMALL_DIM)
        memory.learn("greet", ["say hello", "wave"])
        id_before = memory._name_to_id["greet"]

        memory.clear()
        memory.learn("greet", ["say hello", "wave"])
        id_after = memory._name_to_id["greet"]

        assert id_before != id_after, (
            f"ID collision: learn() before and after clear() produced the same ID {id_before}"
        )

    def test_reinforce_after_clear_raises_key_error(self, tmp_db):
        """reinforce after clear must raise KeyError because the registry was cleared."""
        from llamaindex_velesdb.memory import VelesDBProceduralMemory

        memory = VelesDBProceduralMemory(db_path=tmp_db, dimension=SMALL_DIM)
        memory.learn("greet", ["say hello"])
        memory.clear()
        with pytest.raises(KeyError):
            memory.reinforce("greet", success=False)


# ---------------------------------------------------------------------------
# format_procedural_results helper
# ---------------------------------------------------------------------------


class TestFormatProceduralResults:
    """Tests for the shared format_procedural_results helper."""

    def test_basic_formatting(self):
        """format_procedural_results projects all four expected keys."""
        from velesdb_common.memory import format_procedural_results

        raw = [
            {"name": "proc1", "steps": ["a", "b"], "confidence": 0.9, "score": 0.85},
        ]
        formatted = format_procedural_results(raw)
        assert len(formatted) == 1
        result = formatted[0]
        assert result["name"] == "proc1"
        assert result["steps"] == ["a", "b"]
        assert result["confidence"] == 0.9
        assert result["score"] == 0.85

    def test_empty_input_returns_empty_list(self):
        """format_procedural_results of an empty list returns an empty list."""
        from velesdb_common.memory import format_procedural_results

        assert format_procedural_results([]) == []

    def test_multiple_results_preserved(self):
        """format_procedural_results preserves all items in order."""
        from velesdb_common.memory import format_procedural_results

        raw = [
            {"name": "a", "steps": ["1"], "confidence": 0.8, "score": 0.9},
            {"name": "b", "steps": ["2", "3"], "confidence": 0.6, "score": 0.7},
        ]
        formatted = format_procedural_results(raw)
        assert len(formatted) == 2
        assert formatted[0]["name"] == "a"
        assert formatted[1]["name"] == "b"


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
