"""ID generation utilities shared across VelesDB Python integrations.

.. note::
   **Interop boundary (VelesDB issue #1542).** :func:`stable_hash_id`'s
   default algorithm (SHA-256) does **not** produce the same ``u64`` as
   ``velesdb_core::hash_id`` — VelesDB core's canonical, documented as
   "the single authoritative source" string→u64 hash (FNV-1a) — or as
   ``velesdb-migrate``'s FNV-1a fallback for non-numeric IDs. The same
   logical string therefore maps to a *different* point ID depending on
   whether it entered VelesDB through this package (LangChain, LlamaIndex,
   Haystack) or through core / ``velesdb-migrate``. This is intentional and
   preserved as the default: changing the default here to FNV-1a would
   change the ``u64`` derived from every existing string ID, silently
   orphaning any data already stored through these integrations. Callers
   who need interop with core/``velesdb-migrate``'s IDs for the *same*
   string can opt in with ``stable_hash_id(value, algorithm="fnv1a")`` — see
   that function's docstring for what changes. Full details, including the
   third numeric-preserving semantics used by ``velesdb-migrate``, are in
   ``docs/reference/KNOWN_LIMITATIONS.md`` #12 in the VelesDB repository.
"""

from __future__ import annotations

import hashlib
import secrets
import time

# FNV-1a 64-bit: a public-domain algorithm with standard published constants
# (see http://www.isthe.com/chongo/tech/comp/fnv/). This is a from-scratch,
# MIT-licensed reimplementation of that well-known public algorithm — not a
# copy of VelesDB core's Rust implementation — chosen so that
# `stable_hash_id(v, algorithm="fnv1a")` agrees byte-for-byte with
# `velesdb_core::hash_id(v)` for every `v`. That equivalence is pinned by
# golden vectors published by core, reproduced as literals in
# `tests/test_ids.py` (no Rust import required).
_FNV64_OFFSET_BASIS = 0xCBF29CE484222325
_FNV64_PRIME = 0x100000001B3
_U64_MASK = 0xFFFFFFFFFFFFFFFF


def _fnv1a_64(value: str) -> int:
    """Compute the FNV-1a 64-bit hash of ``value``'s UTF-8 bytes.

    Full unsigned 64-bit output (bit 63 may be set) — unlike the SHA-256
    default, this does not clear the sign bit, so it matches
    ``velesdb_core::hash_id``'s output exactly rather than staying within
    the positive-i64 range.
    """
    digest = _FNV64_OFFSET_BASIS
    for byte in value.encode("utf-8"):
        digest ^= byte
        digest = (digest * _FNV64_PRIME) & _U64_MASK
    return digest


def make_initial_id_counter() -> int:
    """Generate an initial counter value for ID generation.

    Uses the current millisecond timestamp plus a cryptographically secure
    random offset to prevent collisions between concurrent instances or
    process restarts.

    Returns:
        A positive integer suitable as an ID counter seed.
    """
    return int(time.time() * 1000) + secrets.randbelow(9_000_000) + 1_000_000


def stable_hash_id(value: str, *, algorithm: str = "sha256") -> int:
    """Generate a stable numeric ID from a string.

    Python's ``hash()`` is non-deterministic across processes, so this
    function uses an explicit, versioned algorithm for consistent IDs
    across runs.

    Args:
        value: String to hash.
        algorithm: Which hash to use — ``"sha256"`` (default, unchanged) or
            ``"fnv1a"`` (opt-in). See the "Interop boundary" note in this
            module's docstring before switching: the two algorithms produce
            *different* IDs for the same string, so changing the algorithm
            for a store with pre-existing data orphans it (the old IDs are
            no longer derivable from the source strings).

            - ``"sha256"``: SHA-256 of the UTF-8 string, top 8 bytes with
              the sign bit cleared. Positive 63-bit integer ID. This is the
              historical default every VelesDB Python integration has always
              used; existing stores depend on it staying the default.
            - ``"fnv1a"``: FNV-1a 64-bit over the UTF-8 bytes, matching
              ``velesdb_core::hash_id`` and ``velesdb-migrate``'s non-numeric
              fallback byte-for-byte. Full unsigned 64-bit integer ID (bit 63
              may be set). Use this when a string ID must agree with the ID
              core or ``velesdb-migrate`` would derive for the same string —
              e.g. correlating a document ingested via LangChain/LlamaIndex/
              Haystack with the same document migrated via ``velesdb-migrate``.

    Returns:
        A deterministic non-negative integer ID; see `algorithm` above for
        the exact range per algorithm.

    Raises:
        ValueError: if `algorithm` is not ``"sha256"`` or ``"fnv1a"``.
    """
    if algorithm == "sha256":
        hash_bytes = hashlib.sha256(value.encode("utf-8")).digest()
        # Use 8 bytes (64 bits) and clear the sign bit to stay in positive i64 range.
        return int.from_bytes(hash_bytes[:8], byteorder="big") & 0x7FFFFFFFFFFFFFFF
    if algorithm == "fnv1a":
        return _fnv1a_64(value)
    raise ValueError(f"Unknown stable_hash_id algorithm: {algorithm!r} (expected 'sha256' or 'fnv1a')")
