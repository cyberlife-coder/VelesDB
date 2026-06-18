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
