# Using the Python SDK alongside a remote `velesdb-server`

Moved out of [`crates/velesdb-python/README.md`](../../crates/velesdb-python/README.md)
to keep that file under the documentation line budget.

The `velesdb` Python package provides **embedded** (in-process) access to
VelesDB: it opens a database directory on the local filesystem, and there is no
Python client class for a remote server. To talk to a running `velesdb-server`
instance (with optional API key authentication), use plain HTTP requests:

```python
import requests

API_URL = "http://localhost:8080"
API_KEY = "my-secret-key"  # Only needed when server has auth enabled

headers = {"Authorization": f"Bearer {API_KEY}"}

# Search for similar vectors
response = requests.post(
    f"{API_URL}/collections/documents/search",
    json={"vector": [0.1, 0.2, 0.3, 0.4], "top_k": 5},
    headers=headers,
)
results = response.json()
```

When the server has TLS enabled, use `https://` and optionally pass
`verify=False` for self-signed certificates.

See [SERVER_SECURITY.md](SERVER_SECURITY.md) for server authentication and TLS
setup.

---

Last updated: 2026-07-25 · Applies to: velesdb-core 4.3.0
