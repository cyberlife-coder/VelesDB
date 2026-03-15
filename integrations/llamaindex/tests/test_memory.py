"""Tests for VelesDB LlamaIndex Memory — clear() ID collision fix."""

import tempfile

import pytest

velesdb = pytest.importorskip("velesdb")


class TestProceduralMemoryClear:
    """Tests for procedural memory clear() ID collision fix."""

    def test_learn_after_clear_produces_different_id(self):
        """learn() after clear() must produce a different ID than learn() before clear()."""
        from llamaindex_velesdb.memory import VelesDBProceduralMemory

        with tempfile.TemporaryDirectory() as tmpdir:
            memory = VelesDBProceduralMemory(db_path=tmpdir, dimension=4)

            memory.learn("greet", ["say hello", "wave"])
            id_before = memory._name_to_id.get("greet")

            memory.clear()
            memory.learn("greet", ["say hello", "wave"])
            id_after = memory._name_to_id.get("greet")

            assert id_before is not None
            assert id_after is not None
            assert id_before != id_after, (
                f"ID collision: learn() before and after clear() produced the same ID {id_before}"
            )
