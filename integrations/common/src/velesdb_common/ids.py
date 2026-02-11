"""Stable ID generation for VelesDB Python integrations.

Provides deterministic, collision-resistant numeric IDs from string values.
Used by all VelesDB Python integrations to convert string document IDs
to the u64 numeric IDs required by VelesDB core.
"""

import hashlib


def stable_hash_id(value: str) -> int:
    """Generate a stable numeric ID from a string using SHA256.

    Python's hash() is non-deterministic across processes, so we use
    SHA256 truncated to 63 bits (positive i64 range) for stability.

    Args:
        value: String value to hash (typically a document ID or UUID).

    Returns:
        Positive integer ID (0 to 2^63-1), safe for VelesDB u64 storage.

    Raises:
        TypeError: If value is not a string.
    """
    if not isinstance(value, str):
        raise TypeError(f"stable_hash_id expects a string, got {type(value).__name__}")

    digest = hashlib.sha256(value.encode("utf-8")).digest()
    # Take first 8 bytes as unsigned 64-bit, mask to positive i64 range
    raw = int.from_bytes(digest[:8], byteorder="big")
    return raw & 0x7FFFFFFFFFFFFFFF
