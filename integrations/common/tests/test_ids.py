import pytest

from velesdb_common.ids import make_initial_id_counter, stable_hash_id


def test_make_initial_id_counter_returns_positive_int():
    import time
    counter = make_initial_id_counter()
    assert isinstance(counter, int)
    # Contract: ms-timestamp seed + random offset in [1_000_000, 9_999_999].
    # now_ms is sampled after the call, so counter must lie within
    # [now_ms, now_ms + 10_000_000]. Fails for a constant or a pure
    # randbelow() that drops the timestamp anchor.
    now_ms = int(time.time() * 1000)
    assert now_ms <= counter <= now_ms + 10_000_000


def test_make_initial_id_counter_unique():
    counters = {make_initial_id_counter() for _ in range(100)}
    # At least 90% unique (accounting for rare collisions)
    assert len(counters) >= 90


def test_stable_hash_id_deterministic():
    assert stable_hash_id("hello") == stable_hash_id("hello")


def test_stable_hash_id_different_inputs():
    assert stable_hash_id("hello") != stable_hash_id("world")


def test_stable_hash_id_returns_positive_int():
    result = stable_hash_id("test")
    assert isinstance(result, int)
    assert result > 0


# ─────────────────────────────────────────────────────────────
# Issue #1542: opt-in `algorithm="fnv1a"` for interop with
# velesdb_core::hash_id / velesdb-migrate's FNV-1a fallback.
#
# The default MUST stay "sha256" and MUST stay byte-for-byte unchanged
# (backward compatibility for every existing store built via LangChain,
# LlamaIndex, or Haystack). `algorithm="fnv1a"` is new, additive, opt-in
# behavior, verified against golden vectors published by VelesDB core
# (crates/velesdb-core/src/wire/stable_hash_tests.rs and the equivalent
# vectors added to crates/velesdb-migrate and crates/velesdb-memory for the
# same issue) — reproduced here as literals, no Rust import involved.
# ─────────────────────────────────────────────────────────────

# (input, expected velesdb_core::hash_id / hash_id_bytes output)
_CORE_FNV1A_GOLDEN_VECTORS = [
    ("", 0xCBF29CE484222325),
    ("a", 0xAF63DC4C8601EC8C),
    ("foobar", 0x8594_4171_F739_67E8),
    ("tenant:acme", 0x434A088F8B775207),
    # Multi-byte UTF-8: 2-byte (é), 3-byte (CJK), and 4-byte (emoji)
    # sequences must hash over raw UTF-8 bytes, exactly like core.
    ("café", 0x48E8823ACFA40D89),
    ("日本語", 0xEE9EE2B5C854EF87),
    ("emoji:🚀", 0x5063383E8FB557FA),
]


def test_stable_hash_id_default_algorithm_is_sha256_and_unchanged():
    # Backward compatibility: omitting `algorithm` must behave exactly like
    # `algorithm="sha256"` — no existing caller's IDs may shift.
    assert stable_hash_id("hello") == stable_hash_id("hello", algorithm="sha256")


@pytest.mark.parametrize("value,expected", _CORE_FNV1A_GOLDEN_VECTORS)
def test_stable_hash_id_fnv1a_matches_core_golden_vectors(value, expected):
    assert stable_hash_id(value, algorithm="fnv1a") == expected


def test_stable_hash_id_fnv1a_can_exceed_positive_i63_range():
    # Unlike the sha256 default, fnv1a does not clear the sign bit: it must
    # match velesdb_core::hash_id's full unsigned 64-bit output exactly, so
    # bit 63 may legitimately be set.
    result = stable_hash_id("hello", algorithm="fnv1a")
    assert result == 0xA430D84680AABD0B
    assert result > 0x7FFFFFFFFFFFFFFF


def test_stable_hash_id_fnv1a_diverges_from_sha256_default():
    # This is the documented interop boundary itself: the two algorithms
    # must NOT agree for the same input (that's the entire point of the
    # opt-in existing).
    assert stable_hash_id("hello", algorithm="sha256") != stable_hash_id(
        "hello", algorithm="fnv1a"
    )


def test_stable_hash_id_rejects_unknown_algorithm():
    with pytest.raises(ValueError, match="fnv1a"):
        stable_hash_id("hello", algorithm="blake3")
