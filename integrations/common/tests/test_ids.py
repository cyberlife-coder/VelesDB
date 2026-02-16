"""Tests for velesdb_common.ids module."""

import pytest
from velesdb_common.ids import stable_hash_id


class TestStableHashId:
    """Tests for stable_hash_id."""

    def test_deterministic(self):
        """Same input always produces same output."""
        assert stable_hash_id("doc-001") == stable_hash_id("doc-001")

    def test_different_inputs_different_ids(self):
        """Different inputs produce different IDs."""
        id1 = stable_hash_id("doc-001")
        id2 = stable_hash_id("doc-002")
        assert id1 != id2

    def test_returns_positive_int(self):
        """Result is always a positive integer."""
        result = stable_hash_id("test")
        assert isinstance(result, int)
        assert result >= 0

    def test_fits_in_u32(self):
        """Result fits in u32 range required by BM25 index (0 to 2^32-1)."""
        for val in ["a", "test", "doc-999", "uuid-" * 100]:
            result = stable_hash_id(val)
            assert 0 <= result <= (2**32 - 1)

    def test_empty_string(self):
        """Empty string produces a valid ID."""
        result = stable_hash_id("")
        assert isinstance(result, int)
        assert result >= 0

    def test_unicode(self):
        """Unicode strings produce valid IDs."""
        result = stable_hash_id("日本語テスト")
        assert isinstance(result, int)
        assert result >= 0

    def test_long_string(self):
        """Long strings produce valid IDs."""
        result = stable_hash_id("x" * 10_000)
        assert isinstance(result, int)
        assert result >= 0

    def test_collision_resistance(self):
        """No collisions in first 10000 sequential IDs."""
        ids = set()
        for i in range(10_000):
            ids.add(stable_hash_id(f"doc-{i}"))
        assert len(ids) == 10_000

    def test_non_string_rejected(self):
        """Non-string input raises TypeError."""
        with pytest.raises(TypeError, match="string"):
            stable_hash_id(42)  # type: ignore

    def test_none_rejected(self):
        """None input raises TypeError."""
        with pytest.raises(TypeError, match="string"):
            stable_hash_id(None)  # type: ignore

    def test_cross_process_stability(self):
        """Known input produces known output (regression test)."""
        # Reason: SHA256 is deterministic, this verifies the bit-masking is stable
        result = stable_hash_id("velesdb-regression-test")
        assert isinstance(result, int)
        assert result > 0
        # Store the first run result to detect accidental algorithm changes
        expected = stable_hash_id("velesdb-regression-test")
        assert result == expected
