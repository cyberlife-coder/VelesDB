"""ID generation utilities shared across VelesDB Python integrations."""

from __future__ import annotations

import hashlib
import random
import time


def make_initial_id_counter() -> int:
    """Generate an initial counter value for ID generation.

    Uses the current millisecond timestamp plus a large random offset to
    prevent collisions between concurrent instances or process restarts.

    Returns:
        A positive integer suitable as an ID counter seed.
    """
    return int(time.time() * 1000) + random.randint(1_000_000, 9_999_999)


def stable_hash_id(value: str) -> int:
    """Generate a stable numeric ID from a string using SHA-256.

    Python's ``hash()`` is non-deterministic across processes.  This
    function uses SHA-256 for consistent IDs across runs.

    Uses 63 bits from SHA-256 for a very low collision probability while
    keeping a positive integer that is compatible with VelesDB point IDs
    (positive i64 range).

    Args:
        value: String to hash.

    Returns:
        Positive 63-bit integer ID.
    """
    hash_bytes = hashlib.sha256(value.encode("utf-8")).digest()
    # Use 8 bytes (64 bits) and clear the sign bit to stay in positive i64 range.
    return int.from_bytes(hash_bytes[:8], byteorder="big") & 0x7FFFFFFFFFFFFFFF
