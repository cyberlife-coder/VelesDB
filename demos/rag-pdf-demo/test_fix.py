"""Test performance après fix client persistant."""
import httpx
import time

print("Test performance API après fix:")
for i in range(5):
    t0 = time.perf_counter()
    r = httpx.post(
        "http://localhost:8000/search",
        json={"query": "projet", "top_k": 3}
    )
    total = (time.perf_counter() - t0) * 1000
    d = r.json()
    print(f"  Req {i+1}: Total={total:.0f}ms | VelesDB={d.get('search_time_ms', 0):.2f}ms | Embed={d.get('embedding_time_ms', 0):.0f}ms")
