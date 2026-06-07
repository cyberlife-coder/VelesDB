"""
Agent Memory SDK - comprehensive Python binding tests.

Covers all three memory subsystems (semantic, episodic, procedural)
with functional and performance tests.

VELESDB_AVAILABLE / pytestmark / temp_db fixture are provided by conftest.py.

Run with: pytest tests/test_agent_memory.py -v
"""

import tempfile
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
# TTL / Eviction
# =========================================================================


class TestTtlAndEviction:
    """Tests for TTL helpers and eviction controls on AgentMemory."""

    def test_store_with_ttl_immediately_queryable(self, memory):
        """store_with_ttl stores a fact that is still queryable before expiry.

        Note: the TTL registry lives on the SemanticMemory instance, so the
        same ``semantic`` handle must be reused for store and query.
        """
        semantic = memory.semantic
        semantic.store_with_ttl(1, "ephemeral", [0.1, 0.2, 0.3, 0.4], 3600)
        results = semantic.query([0.1, 0.2, 0.3, 0.4], top_k=1)
        assert len(results) == 1
        assert results[0]["content"] == "ephemeral"

    def test_store_with_ttl_zero_expires_on_query(self, memory):
        """A TTL of 0 seconds makes the fact expire and be filtered on query.

        The same ``semantic`` handle is reused so the in-memory TTL registry
        set by ``store_with_ttl`` is honoured by the subsequent ``query``.
        """
        semantic = memory.semantic
        semantic.store_with_ttl(1, "already stale", [0.1, 0.2, 0.3, 0.4], 0)
        results = semantic.query([0.1, 0.2, 0.3, 0.4], top_k=10)
        ids = [r["id"] for r in results]
        assert 1 not in ids

    def test_set_ttl_helpers_do_not_raise(self, memory):
        """set_*_ttl helpers accept ids/seconds without raising."""
        memory.semantic.store(1, "fact", [0.1, 0.2, 0.3, 0.4])
        memory.episodic.record(2, "event", int(time.time()))
        memory.procedural.learn(3, "proc", ["s1"])
        memory.set_semantic_ttl(1, 3600)
        memory.set_episodic_ttl(2, 3600)
        memory.set_procedural_ttl(3, 3600)

    def test_auto_expire_returns_stats_dict(self, memory):
        """auto_expire returns a dict with the expected counter keys."""
        result = memory.auto_expire()
        for key in (
            "semantic_expired",
            "episodic_expired",
            "procedural_expired",
            "episodic_consolidated",
            "procedural_evicted",
        ):
            assert key in result
            assert isinstance(result[key], int)

    def test_auto_expire_removes_expired_entry(self, memory):
        """set_semantic_ttl(0) + auto_expire deletes the entry from storage."""
        memory.semantic.store(1, "to expire", [0.1, 0.2, 0.3, 0.4])
        memory.set_semantic_ttl(1, 0)
        result = memory.auto_expire()
        assert result["semantic_expired"] >= 1

    def test_evict_low_confidence_procedures(self, memory):
        """Procedures below the confidence threshold are evicted."""
        memory.procedural.learn(1, "reliable", ["s1"], [0.5, 0.5, 0.0, 0.0], 0.9)
        memory.procedural.learn(2, "weak", ["s1"], [0.5, 0.5, 0.0, 0.0], 0.2)
        evicted = memory.evict_low_confidence_procedures(0.5)
        assert evicted == 1
        ids = [p["id"] for p in memory.procedural.list_all()]
        assert 1 in ids
        assert 2 not in ids


# =========================================================================
# Serialize / Deserialize
# =========================================================================


class TestSemanticSerialization:
    """Tests for SemanticMemory.serialize / deserialize round-trip."""

    def test_serialize_returns_bytes(self, memory):
        """serialize returns a bytes blob."""
        memory.semantic.store(1, "fact", [0.1, 0.2, 0.3, 0.4])
        blob = memory.semantic.serialize()
        assert isinstance(blob, (bytes, bytearray))

    def test_serialize_deserialize_round_trip(self, temp_db):
        """deserialize restores facts captured by serialize."""
        source = temp_db.agent_memory(dimension=4)
        source.semantic.store(1, "restored fact", [0.1, 0.2, 0.3, 0.4])
        blob = source.semantic.serialize()

        target = temp_db.agent_memory(dimension=4)
        target.semantic.delete(1)
        target.semantic.deserialize(blob)
        results = target.semantic.query([0.1, 0.2, 0.3, 0.4], top_k=1)
        assert len(results) == 1
        assert results[0]["content"] == "restored fact"


# =========================================================================
# Snapshots
# =========================================================================


class TestSnapshots:
    """Tests for the versioned snapshot suite on AgentMemory."""

    def _memory_with_snapshots(self, temp_db, snapshot_dir):
        return temp_db.agent_memory(dimension=4, snapshot_dir=snapshot_dir)

    def test_snapshot_returns_version(self, temp_db):
        """snapshot returns a monotonically increasing version number."""
        with tempfile.TemporaryDirectory() as snap_dir:
            mem = self._memory_with_snapshots(temp_db, snap_dir)
            mem.semantic.store(1, "v1 fact", [0.1, 0.2, 0.3, 0.4])
            v1 = mem.snapshot()
            v2 = mem.snapshot()
            assert v2 > v1

    def test_list_snapshot_versions(self, temp_db):
        """list_snapshot_versions reports every created version."""
        with tempfile.TemporaryDirectory() as snap_dir:
            mem = self._memory_with_snapshots(temp_db, snap_dir)
            mem.semantic.store(1, "fact", [0.1, 0.2, 0.3, 0.4])
            v1 = mem.snapshot()
            v2 = mem.snapshot()
            versions = mem.list_snapshot_versions()
            assert v1 in versions
            assert v2 in versions

    def test_load_latest_snapshot_returns_version(self, temp_db):
        """load_latest_snapshot returns the version it restored.

        The snapshot captures whatever the AgentMemory's own subsystems track;
        here we assert the load mechanics (version returned, no error) rather
        than getter-stored round-trips, which use independent TTL/tracking.
        """
        with tempfile.TemporaryDirectory() as snap_dir:
            mem = self._memory_with_snapshots(temp_db, snap_dir)
            mem.semantic.store(1, "snapshotted", [0.1, 0.2, 0.3, 0.4])
            created = mem.snapshot()
            loaded = mem.load_latest_snapshot()
            assert loaded == created

    def test_load_snapshot_version_does_not_raise(self, temp_db):
        """load_snapshot_version restores a known version without error."""
        with tempfile.TemporaryDirectory() as snap_dir:
            mem = self._memory_with_snapshots(temp_db, snap_dir)
            mem.semantic.store(1, "first", [0.1, 0.2, 0.3, 0.4])
            v1 = mem.snapshot()
            mem.snapshot()
            mem.load_snapshot_version(v1)

    def test_load_unknown_snapshot_version_raises(self, temp_db):
        """Loading a non-existent version raises."""
        with tempfile.TemporaryDirectory() as snap_dir:
            mem = self._memory_with_snapshots(temp_db, snap_dir)
            mem.semantic.store(1, "fact", [0.1, 0.2, 0.3, 0.4])
            mem.snapshot()
            with pytest.raises(Exception):
                mem.load_snapshot_version(999_999)

    def test_snapshot_without_dir_raises(self, memory):
        """snapshot raises when no snapshot_dir was configured."""
        with pytest.raises(RuntimeError):
            memory.snapshot()


# =========================================================================
# VelesQL bridges
# =========================================================================


class TestVelesQlBridges:
    """Tests for query_semantic / query_episodic / query_procedural."""

    def test_query_semantic(self, memory):
        """query_semantic runs VelesQL against the semantic collection."""
        memory.semantic.store(1, "the sky is blue", [1.0, 0.0, 0.0, 0.0])
        memory.semantic.store(2, "grass is green", [0.0, 1.0, 0.0, 0.0])
        results = memory.query_semantic(
            "SELECT * FROM _semantic_memory WHERE vector NEAR $v LIMIT 5",
            params={"v": [1.0, 0.0, 0.0, 0.0]},
        )
        assert len(results) >= 1
        assert results[0]["id"] == 1

    def test_query_episodic(self, memory):
        """query_episodic runs VelesQL against the episodic collection."""
        memory.episodic.record(1, "event one", 1_000_000, [1.0, 0.0, 0.0, 0.0])
        memory.episodic.record(2, "event two", 2_000_000, [0.0, 1.0, 0.0, 0.0])
        results = memory.query_episodic(
            "SELECT * FROM _episodic_memory WHERE vector NEAR $v LIMIT 5",
            params={"v": [1.0, 0.0, 0.0, 0.0]},
        )
        assert len(results) >= 1
        assert results[0]["id"] == 1

    def test_query_procedural(self, memory):
        """query_procedural runs a scan VelesQL against the procedural collection."""
        memory.procedural.learn(1, "proc1", ["s1"], [1.0, 0.0, 0.0, 0.0], 0.8)
        results = memory.query_procedural(
            "SELECT * FROM _procedural_memory LIMIT 10"
        )
        assert len(results) == 1

    def test_query_semantic_empty(self, memory):
        """query_semantic on empty memory returns an empty list."""
        results = memory.query_semantic(
            "SELECT * FROM _semantic_memory WHERE vector NEAR $v LIMIT 5",
            params={"v": [1.0, 0.0, 0.0, 0.0]},
        )
        assert results == []

    def test_query_invalid_sql_raises(self, memory):
        """Invalid VelesQL raises an exception."""
        with pytest.raises(Exception):
            memory.query_semantic("THIS IS NOT SQL")


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
        """Semantic store sustains a sane throughput floor (dim=4).

        Each store is a persisted upsert (~10 ms on commodity hardware), so the
        floor is a regression guard with headroom, not a tuned target — a value
        near 100 facts/sec is normal; this only trips on a pathological (>3x)
        regression.
        """
        n = 200
        emb = [0.25, 0.25, 0.25, 0.25]
        t0 = time.perf_counter()
        for i in range(n):
            memory.semantic.store(100 + i, f"fact {i}", emb)
        rate = n / (time.perf_counter() - t0)
        assert rate > 30, f"Store rate {rate:.0f} facts/sec < 30 (regression floor)"

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
