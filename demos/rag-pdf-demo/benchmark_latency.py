"""Benchmark détaillé de la latence VelesDB."""
import httpx
import time
import asyncio

print("=" * 50)
print("ANALYSE DETAILLEE LATENCE VELESDB")
print("=" * 50)

# Test 1: Client SYNC réutilisé
print("\n1. Test SYNC avec CLIENT REUTILISE (base_url):")
client = httpx.Client(base_url="http://localhost:8080", timeout=30.0)
for i in range(5):
    t0 = time.perf_counter()
    r = client.get("/health")
    latency = (time.perf_counter() - t0) * 1000
    print(f"   Req {i+1}: {latency:.2f}ms")
client.close()

# Test 2: Search avec client réutilisé
print("\n2. Test SEARCH avec client réutilisé:")
vector = [0.1] * 384
client = httpx.Client(base_url="http://localhost:8080", timeout=30.0)
for i in range(5):
    t0 = time.perf_counter()
    r = client.post("/collections/rag_documents/search", json={"vector": vector, "top_k": 5})
    latency = (time.perf_counter() - t0) * 1000
    print(f"   Search {i+1}: {latency:.2f}ms - {len(r.json().get('results', []))} results")
client.close()

# Test 3: Client ASYNC réutilisé (comme notre fix)
async def test_async():
    print("\n3. Test ASYNC avec CLIENT REUTILISE (comme le fix):")
    client = httpx.AsyncClient(base_url="http://localhost:8080", timeout=30.0)
    for i in range(5):
        t0 = time.perf_counter()
        r = await client.get("/health")
        latency = (time.perf_counter() - t0) * 1000
        print(f"   Async Req {i+1}: {latency:.2f}ms")
    
    print("\n4. Test ASYNC SEARCH:")
    for i in range(5):
        t0 = time.perf_counter()
        r = await client.post("/collections/rag_documents/search", json={"vector": vector, "top_k": 5})
        latency = (time.perf_counter() - t0) * 1000
        print(f"   Async Search {i+1}: {latency:.2f}ms")
    await client.aclose()

asyncio.run(test_async())

print("\n" + "=" * 50)
print("FIN BENCHMARK - Client persistant = performances optimales")
print("=" * 50)
