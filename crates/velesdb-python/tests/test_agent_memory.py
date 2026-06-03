"""
Agent Memory SDK - comprehensive Python binding tests.

Covers all three memory subsystems (semantic, episodic, procedural)
with functional and performance tests.

VELESDB_AVAILABLE / pytestmark / temp_db fixture are provided by conftest.py.

Run with: pytest tests/test_agent_memory.py -v
"""

import time

import pytest

from conftest import _SKIP_NO_BINDINGS

pytestmark = _SKIP_NO_BINDINGS


@pytest.fixture
def memory(temp_db):
    """Create an AgentMemory with dimension=4 for testing."""
    return temp_db.agent_memory(dimension=4)


# =========================================================================
# Semantic Memory
# =========================================================================


class TestSemanticMemory:
    """Tests for SemanticMemory: store, query, delete."""

    def test_store_and_query(self, memory):
        """Store a fact and retrieve it by similarity."""
        memory.semantic.store(1, "Paris is the capital of France", [0.1, 0.2, 0.3, 0.4])
        results = memory.semantic.query([0.1, 0.2, 0.3, 0.4], top_k=1)
        assert len(results) == 1
        assert results[0]["id"] == 1
        assert results[0]["content"] == "Paris is the capital of France"
        assert 0.0 <= results[0]["score"] <= 1.0

    def test_query_returns_top_k_ordered(self, memory):
        """Query must return results ordered by similarity (best first)."""
        memory.semantic.store(1, "very similar", [0.9, 0.1, 0.0, 0.0])
        memory.semantic.store(2, "somewhat similar", [0.5, 0.5, 0.0, 0.0])
        memory.semantic.store(3, "different", [0.0, 0.0, 0.9, 0.1])
        results = memory.semantic.query([0.9, 0.1, 0.0, 0.0], top_k=3)
        assert len(results) == 3
        assert results[0]["id"] == 1  # most similar first

    def test_upsert_overwrites(self, memory):
        """Storing with the same ID overwrites the previous fact."""
        memory.semantic.store(1, "original", [0.1, 0.2, 0.3, 0.4])
        memory.semantic.store(1, "updated", [0.1, 0.2, 0.3, 0.4])
        results = memory.semantic.query([0.1, 0.2, 0.3, 0.4], top_k=1)
        assert results[0]["content"] == "updated"

    def test_delete(self, memory):
        """Delete removes a fact from results."""
        memory.semantic.store(1, "fact A", [0.1, 0.2, 0.3, 0.4])
        memory.semantic.store(2, "fact B", [0.4, 0.3, 0.2, 0.1])
        memory.semantic.delete(1)
        results = memory.semantic.query([0.1, 0.2, 0.3, 0.4], top_k=10)
        ids = [r["id"] for r in results]
        assert 1 not in ids
        assert 2 in ids

    def test_query_empty(self, memory):
        """Query on empty memory returns empty list."""
        results = memory.semantic.query([0.1, 0.2, 0.3, 0.4], top_k=5)
        assert results == []

    def test_repr(self, memory):
        """repr shows dimension."""
        assert "4" in repr(memory.semantic)


# =========================================================================
# Episodic Memory
# =========================================================================


class TestEpisodicMemory:
    """Tests for EpisodicMemory: record, recent, recall_similar, older_than, delete."""

    def test_record_and_recent(self, memory):
        """Record events and retrieve recent ones."""
        now = int(time.time())
        memory.episodic.record(1, "event A", now - 100)
        memory.episodic.record(2, "event B", now)
        events = memory.episodic.recent(limit=10)
        assert len(events) == 2
        # Most recent first
        assert events[0]["id"] == 2

    def test_recent_with_since(self, memory):
        """recent(since=...) filters events before the threshold."""
        now = int(time.time())
        memory.episodic.record(1, "old event", now - 7200)
        memory.episodic.record(2, "recent event", now)
        events = memory.episodic.recent(limit=10, since=now - 3600)
        assert len(events) == 1
        assert events[0]["id"] == 2

    def test_recall_similar(self, memory):
        """recall_similar finds events by embedding similarity."""
        now = int(time.time())
        memory.episodic.record(1, "geo question", now, [0.9, 0.1, 0.0, 0.0])
        memory.episodic.record(2, "math question", now, [0.0, 0.0, 0.9, 0.1])
        results = memory.episodic.recall_similar([0.9, 0.1, 0.0, 0.0], top_k=2)
        assert len(results) == 2
        assert results[0]["id"] == 1  # most similar first
        assert "score" in results[0]

    def test_older_than(self, memory):
        """older_than returns events before the given timestamp."""
        now = int(time.time())
        memory.episodic.record(1, "very old", now - 86400)
        memory.episodic.record(2, "old", now - 3600)
        memory.episodic.record(3, "fresh", now)
        old = memory.episodic.older_than(before=now - 1800, limit=10)
        ids = [e["id"] for e in old]
        assert 1 in ids
        assert 2 in ids
        assert 3 not in ids

    def test_delete(self, memory):
        """Delete removes an event from recent results."""
        now = int(time.time())
        memory.episodic.record(1, "event A", now)
        memory.episodic.record(2, "event B", now)
        memory.episodic.delete(1)
        events = memory.episodic.recent(limit=10)
        ids = [e["id"] for e in events]
        assert 1 not in ids
        assert 2 in ids

    def test_record_without_embedding(self, memory):
        """Record without embedding still works for temporal queries."""
        now = int(time.time())
        memory.episodic.record(1, "no embedding event", now)
        events = memory.episodic.recent(limit=1)
        assert len(events) == 1
        assert events[0]["description"] == "no embedding event"

    def test_repr(self, memory):
        """repr shows dimension."""
        assert "4" in repr(memory.episodic)


# =========================================================================
# Procedural Memory
# =========================================================================


class TestProceduralMemory:
    """Tests for ProceduralMemory: learn, recall, reinforce, list_all, delete."""

    def test_learn_and_recall(self, memory):
        """Learn a procedure and recall it by similarity."""
        memory.procedural.learn(1, "greet", ["wave", "say hi"], [0.5, 0.5, 0.0, 0.0], 0.8)
        matches = memory.procedural.recall([0.5, 0.5, 0.0, 0.0], top_k=1)
        assert len(matches) == 1
        assert matches[0]["name"] == "greet"
        assert matches[0]["steps"] == ["wave", "say hi"]
        assert matches[0]["confidence"] == pytest.approx(0.8, abs=0.01)

    def test_recall_min_confidence_filter(self, memory):
        """recall with min_confidence filters low-confidence procedures."""
        memory.procedural.learn(1, "reliable", ["step1"], [0.5, 0.5, 0.0, 0.0], 0.9)
        memory.procedural.learn(2, "unreliable", ["step1"], [0.5, 0.5, 0.0, 0.0], 0.2)
        matches = memory.procedural.recall([0.5, 0.5, 0.0, 0.0], top_k=10, min_confidence=0.5)
        ids = [m["id"] for m in matches]
        assert 1 in ids
        assert 2 not in ids

    @pytest.mark.parametrize(
        "success,direction",
        [(True, "above"), (False, "below")],
        ids=["reinforce_success", "reinforce_failure"],
    )
    def test_reinforce(self, memory, success, direction):
        """reinforce() changes confidence in the expected direction."""
        memory.procedural.learn(1, "proc", ["s1"], [0.5, 0.5, 0.0, 0.0], 0.5)
        memory.procedural.reinforce(1, success=success)
        matches = memory.procedural.recall([0.5, 0.5, 0.0, 0.0], top_k=1)
        confidence = matches[0]["confidence"]
        if direction == "above":
            assert confidence > 0.5, (
                f"reinforce(success=True) should increase confidence above 0.5, got {confidence}"
            )
        else:
            assert confidence < 0.5, (
                f"reinforce(success=False) should decrease confidence below 0.5, got {confidence}"
            )

    def test_list_all(self, memory):
        """list_all returns all stored procedures."""
        memory.procedural.learn(1, "proc A", ["s1"], [0.5, 0.5, 0.0, 0.0], 0.7)
        memory.procedural.learn(2, "proc B", ["s2"], [0.1, 0.1, 0.1, 0.1], 0.9)
        all_procs = memory.procedural.list_all()
        assert len(all_procs) == 2
        names = {p["name"] for p in all_procs}
        assert names == {"proc A", "proc B"}

    def test_delete(self, memory):
        """Delete removes a procedure from list_all results."""
        memory.procedural.learn(1, "proc A", ["s1"], [0.5, 0.5, 0.0, 0.0], 0.7)
        memory.procedural.learn(2, "proc B", ["s2"], [0.1, 0.1, 0.1, 0.1], 0.9)
        memory.procedural.delete(1)
        all_procs = memory.procedural.list_all()
        ids = [p["id"] for p in all_procs]
        assert 1 not in ids
        assert 2 in ids

    def test_learn_without_embedding(self, memory):
        """Learn without embedding still works (uses zero vector internally)."""
        memory.procedural.learn(1, "no_emb", ["step1"])
        all_procs = memory.procedural.list_all()
        assert len(all_procs) == 1
        assert all_procs[0]["name"] == "no_emb"

    def test_repr(self, memory):
        """repr shows dimension."""
        assert "4" in repr(memory.procedural)


# =========================================================================
# AgentMemory top-level
# =========================================================================


class TestAgentMemory:
    """Tests for AgentMemory facade."""

    def test_dimension(self, memory):
        """dimension property returns the configured dimension."""
        assert memory.dimension == 4

    def test_repr(self, memory):
        """repr shows dimension."""
        assert "4" in repr(memory)

    def test_multiple_instances_share_data(self, temp_db):
        """Two AgentMemory instances on the same DB share data."""
        mem1 = temp_db.agent_memory(dimension=4)
        mem2 = temp_db.agent_memory(dimension=4)
        mem1.semantic.store(1, "shared fact", [0.1, 0.2, 0.3, 0.4])
        results = mem2.semantic.query([0.1, 0.2, 0.3, 0.4], top_k=1)
        assert len(results) == 1
        assert results[0]["content"] == "shared fact"


# =========================================================================
# Performance
# =========================================================================


@pytest.mark.slow
class TestAgentMemoryPerformance:
    """Performance tests: ensure latency stays within expected bounds.

    Marked ``slow``: these assert on wall-clock throughput/latency and are
    sensitive to machine load, so they are excluded from the deterministic
    gate (``pytest -m "not slow"``). Run explicitly with ``pytest -m slow``.
    """

    def test_semantic_store_throughput(self, memory):
        """Semantic store must sustain >100 facts/sec (dim=4)."""
        n = 200
        emb = [0.25, 0.25, 0.25, 0.25]
        t0 = time.perf_counter()
        for i in range(n):
            memory.semantic.store(100 + i, f"fact {i}", emb)
        rate = n / (time.perf_counter() - t0)
        assert rate > 100, f"Store rate {rate:.0f} facts/sec < 100"

    def test_semantic_query_latency(self, memory):
        """Semantic query p99 must be < 5ms on 500 facts (dim=4)."""
        emb = [0.25, 0.25, 0.25, 0.25]
        for i in range(500):
            memory.semantic.store(i, f"fact {i}", emb)

        lats = []
        for _ in range(50):
            t0 = time.perf_counter()
            memory.semantic.query(emb, top_k=10)
            lats.append((time.perf_counter() - t0) * 1e6)
        lats.sort()
        p99 = lats[int(len(lats) * 0.99)]
        assert p99 < 5000, f"Query p99 {p99:.0f}us > 5ms"

    def test_episodic_recent_latency(self, memory):
        """Episodic recent() must be < 1ms on 200 events."""
        now = int(time.time())
        for i in range(200):
            memory.episodic.record(i, f"event {i}", now - i)

        lats = []
        for _ in range(50):
            t0 = time.perf_counter()
            memory.episodic.recent(limit=10)
            lats.append((time.perf_counter() - t0) * 1e6)
        lats.sort()
        p99 = lats[int(len(lats) * 0.99)]
        assert p99 < 1000, f"Recent p99 {p99:.0f}us > 1ms"
