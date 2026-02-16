"""Stable ID generation for VelesDB Python integrations.

Provides deterministic, collision-resistant numeric IDs from string values.
Used by all VelesDB Python integrations to convert string document IDs
to numeric IDs compatible with all VelesDB indexes.
"""

import hashlib

MAX_VELESDB_ID = 0xFFFFFFFF  # BM25 uses RoaringBitmap (u32 document IDs)


def stable_hash_id(value: str) -> int:
    """Generate a stable numeric ID from a string using SHA256.

    Python's hash() is non-deterministic across processes, so we use
    SHA256 truncated to 32 bits for deterministic cross-process stability.

    Args:
        value: String value to hash (typically a document ID or UUID).

    Returns:
        Positive integer ID (0 to 2^32-1), safe for BM25 and vector indexes.

    Raises:
        TypeError: If value is not a string.
    """
    if not isinstance(value, str):
        raise TypeError(f"stable_hash_id expects a string, got {type(value).__name__}")

    digest = hashlib.sha256(value.encode("utf-8")).digest()
    # Take first 4 bytes as unsigned 32-bit for BM25 compatibility.
    raw = int.from_bytes(digest[:4], byteorder="big")
    return raw & MAX_VELESDB_ID
