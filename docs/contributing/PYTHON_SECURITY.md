# Python Security Guidelines for VelesDB Integrations

## ⚠️ Critical: Avoid Python `hash()` for Persistence/IDs

### The Problem

Python's built-in `hash()` function is **non-deterministic across processes**:

```python
# ❌ WRONG - Different result each Python run
node_id = hash(node.node_id) & 0x7FFFFFFFFFFFFFFF
content_hash = hash(doc.page_content[:200])
```

This causes:
- **Data corruption**: IDs change between runs, breaking graph edges
- **Non-reproducible results**: Same input → different output
- **Silent failures**: Tests pass (single process) but production fails

### Why It Happens

Python enables **hash randomization** by default (since 3.3) via `PYTHONHASHSEED`:
- Security feature against hash collision attacks
- Each process gets a random seed
- `hash("hello")` returns different values across runs

### The Solution

Use `hashlib` for deterministic, collision-resistant hashing:

```python
import hashlib

# ✅ CORRECT - Deterministic SHA256
def generate_stable_id(content: str) -> int:
    """Generate a stable numeric ID from content."""
    hash_bytes = hashlib.sha256(content.encode("utf-8")).digest()
    return int.from_bytes(hash_bytes[:8], byteorder="big")

# ✅ CORRECT - Deterministic deduplication
def deduplicate_docs(docs: List[Document]) -> List[Document]:
    seen = set()
    unique = []
    for doc in docs:
        content_hash = hashlib.sha256(
            doc.page_content[:200].encode("utf-8")
        ).hexdigest()
        if content_hash not in seen:
            seen.add(content_hash)
            unique.append(doc)
    return unique
```

## Dangerous Patterns to Avoid

| Pattern | Problem | Solution |
|---------|---------|----------|
| `hash(str)` | Non-deterministic | `hashlib.sha256(str.encode()).hexdigest()` |
| `id(obj)` | Memory address, not stable | Use explicit IDs or UUID |
| `set()` iteration order | Non-deterministic in older Python | Use `sorted()` or `dict.fromkeys()` |
| `dict` iteration order | Was non-deterministic < 3.7 | Python 3.7+ is fine |

## Pre-commit Hook

The pre-commit hook automatically checks for dangerous `hash()` usage:

```bash
# Blocked patterns:
hash(something)  # Unless in hashlib context

# Allowed patterns:
hashlib.sha256(...)
obj.__hash__()   # Magic method definition
# hash comment
```

## Testing for Determinism

Add this test to verify your code is deterministic:

```python
import subprocess
import sys

def test_deterministic_ids():
    """Verify IDs are stable across Python processes."""
    code = '''
from your_module import generate_id
print(generate_id("test_input"))
'''
    results = set()
    for _ in range(5):
        result = subprocess.check_output([sys.executable, "-c", code])
        results.add(result.strip())
    
    assert len(results) == 1, f"Non-deterministic: got {len(results)} different values"
```

## References

- [Python hash randomization (PEP 456)](https://peps.python.org/pep-0456/)
- [hashlib documentation](https://docs.python.org/3/library/hashlib.html)
