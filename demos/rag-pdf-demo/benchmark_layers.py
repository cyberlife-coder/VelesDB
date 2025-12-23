"""
Benchmark multi-couches pour VelesDB RAG Demo
Teste la stabilité des temps de réponse par couche:
1. TCP Connection
2. HTTP Client Creation
3. HTTP Request (health)
4. VelesDB Search
5. Embedding Generation
6. Full RAG Pipeline
"""
import asyncio
import socket
import statistics
import time
from dataclasses import dataclass

import httpx

# Configuration
VELESDB_URL = "http://localhost:8080"
FASTAPI_URL = "http://localhost:8000"
ITERATIONS = 500
VECTOR_DIM = 384


@dataclass
class BenchmarkResult:
    """Résultat d'un benchmark."""
    name: str
    times_ms: list[float]
    
    @property
    def min(self) -> float:
        return min(self.times_ms)
    
    @property
    def max(self) -> float:
        return max(self.times_ms)
    
    @property
    def mean(self) -> float:
        return statistics.mean(self.times_ms)
    
    @property
    def median(self) -> float:
        return statistics.median(self.times_ms)
    
    @property
    def stdev(self) -> float:
        return statistics.stdev(self.times_ms) if len(self.times_ms) > 1 else 0
    
    @property
    def p95(self) -> float:
        sorted_times = sorted(self.times_ms)
        idx = int(len(sorted_times) * 0.95)
        return sorted_times[min(idx, len(sorted_times) - 1)]


def print_header(title: str):
    print(f"\n{'=' * 60}")
    print(f"  {title}")
    print(f"{'=' * 60}")


def print_result(result: BenchmarkResult):
    print(f"\n📊 {result.name}")
    print(f"   Iterations: {len(result.times_ms)}")
    print(f"   Min:    {result.min:8.2f} ms")
    print(f"   Max:    {result.max:8.2f} ms")
    print(f"   Mean:   {result.mean:8.2f} ms")
    print(f"   Median: {result.median:8.2f} ms")
    print(f"   StdDev: {result.stdev:8.2f} ms")
    print(f"   P95:    {result.p95:8.2f} ms")
    
    # Stabilité indicator
    cv = (result.stdev / result.mean * 100) if result.mean > 0 else 0
    if cv < 10:
        stability = "✅ Très stable"
    elif cv < 25:
        stability = "⚠️  Acceptable"
    else:
        stability = "❌ Instable"
    print(f"   CV:     {cv:8.1f} % ({stability})")


# ============================================================
# LAYER 1: TCP Connection
# ============================================================
def bench_tcp_connection(host: str, port: int, iterations: int) -> BenchmarkResult:
    times = []
    for _ in range(iterations):
        t0 = time.perf_counter()
        sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        sock.connect((host, port))
        sock.close()
        times.append((time.perf_counter() - t0) * 1000)
    return BenchmarkResult("Layer 1: TCP Connection", times)


# ============================================================
# LAYER 2: HTTP Client Creation
# ============================================================
def bench_http_client_creation(iterations: int) -> BenchmarkResult:
    times = []
    for _ in range(iterations):
        t0 = time.perf_counter()
        client = httpx.Client(base_url=VELESDB_URL, timeout=30.0)
        client.close()
        times.append((time.perf_counter() - t0) * 1000)
    return BenchmarkResult("Layer 2: HTTP Client Creation", times)


# ============================================================
# LAYER 3: HTTP Request (with persistent client)
# ============================================================
def bench_http_request_persistent(iterations: int) -> BenchmarkResult:
    times = []
    client = httpx.Client(base_url=VELESDB_URL, timeout=30.0)
    # Warm-up
    client.get("/health")
    
    for _ in range(iterations):
        t0 = time.perf_counter()
        client.get("/health")
        times.append((time.perf_counter() - t0) * 1000)
    client.close()
    return BenchmarkResult("Layer 3: HTTP Request (persistent client)", times)


# ============================================================
# LAYER 4: VelesDB Search
# ============================================================
def bench_velesdb_search(iterations: int) -> BenchmarkResult:
    times = []
    vector = [0.1] * VECTOR_DIM
    client = httpx.Client(base_url=VELESDB_URL, timeout=30.0)
    # Warm-up
    client.post("/collections/rag_documents/search", json={"vector": vector, "top_k": 5})
    
    for _ in range(iterations):
        t0 = time.perf_counter()
        client.post("/collections/rag_documents/search", json={"vector": vector, "top_k": 5})
        times.append((time.perf_counter() - t0) * 1000)
    client.close()
    return BenchmarkResult("Layer 4: VelesDB Search (384 dims)", times)


# ============================================================
# LAYER 5: Full API Search (FastAPI -> Embedding -> VelesDB)
# ============================================================
def bench_full_api_search(iterations: int) -> BenchmarkResult:
    times = []
    embedding_times = []
    search_times = []
    
    client = httpx.Client(base_url=FASTAPI_URL, timeout=60.0)
    # Warm-up
    client.post("/search", json={"query": "test warmup", "top_k": 3})
    
    for _ in range(iterations):
        t0 = time.perf_counter()
        r = client.post("/search", json={"query": "machine learning", "top_k": 5})
        total = (time.perf_counter() - t0) * 1000
        times.append(total)
        
        data = r.json()
        embedding_times.append(data.get("embedding_time_ms", 0))
        search_times.append(data.get("search_time_ms", 0))
    
    client.close()
    
    # Print sub-metrics
    print(f"\n   📈 Sub-metrics from API response:")
    print(f"      Embedding: mean={statistics.mean(embedding_times):.2f}ms, stdev={statistics.stdev(embedding_times):.2f}ms")
    print(f"      VelesDB:   mean={statistics.mean(search_times):.2f}ms, stdev={statistics.stdev(search_times):.2f}ms")
    
    return BenchmarkResult("Layer 5: Full API Search (FastAPI+Embed+VelesDB)", times)


# ============================================================
# ASYNC LAYER: Async HTTP Client
# ============================================================
async def bench_async_http(iterations: int) -> BenchmarkResult:
    times = []
    async with httpx.AsyncClient(base_url=VELESDB_URL, timeout=30.0) as client:
        # Warm-up
        await client.get("/health")
        
        for _ in range(iterations):
            t0 = time.perf_counter()
            await client.get("/health")
            times.append((time.perf_counter() - t0) * 1000)
    
    return BenchmarkResult("Layer 3b: Async HTTP Request", times)


async def bench_async_search(iterations: int) -> BenchmarkResult:
    times = []
    vector = [0.1] * VECTOR_DIM
    async with httpx.AsyncClient(base_url=VELESDB_URL, timeout=30.0) as client:
        # Warm-up
        await client.post("/collections/rag_documents/search", json={"vector": vector, "top_k": 5})
        
        for _ in range(iterations):
            t0 = time.perf_counter()
            await client.post("/collections/rag_documents/search", json={"vector": vector, "top_k": 5})
            times.append((time.perf_counter() - t0) * 1000)
    
    return BenchmarkResult("Layer 4b: Async VelesDB Search", times)


# ============================================================
# MAIN
# ============================================================
def main():
    print_header("VelesDB RAG Demo - Benchmark Multi-Couches")
    print(f"Iterations par test: {ITERATIONS}")
    print(f"VelesDB URL: {VELESDB_URL}")
    print(f"FastAPI URL: {FASTAPI_URL}")
    
    results = []
    
    # Layer 1: TCP
    print("\n⏳ Testing Layer 1: TCP Connection...")
    results.append(bench_tcp_connection("localhost", 8080, ITERATIONS))
    print_result(results[-1])
    
    # Layer 2: HTTP Client Creation
    print("\n⏳ Testing Layer 2: HTTP Client Creation...")
    results.append(bench_http_client_creation(ITERATIONS))
    print_result(results[-1])
    
    # Layer 3: HTTP Request
    print("\n⏳ Testing Layer 3: HTTP Request (persistent)...")
    results.append(bench_http_request_persistent(ITERATIONS))
    print_result(results[-1])
    
    # Layer 3b: Async HTTP
    print("\n⏳ Testing Layer 3b: Async HTTP Request...")
    results.append(asyncio.run(bench_async_http(ITERATIONS)))
    print_result(results[-1])
    
    # Layer 4: VelesDB Search
    print("\n⏳ Testing Layer 4: VelesDB Search...")
    results.append(bench_velesdb_search(ITERATIONS))
    print_result(results[-1])
    
    # Layer 4b: Async Search
    print("\n⏳ Testing Layer 4b: Async VelesDB Search...")
    results.append(asyncio.run(bench_async_search(ITERATIONS)))
    print_result(results[-1])
    
    # Layer 5: Full API
    print("\n⏳ Testing Layer 5: Full API Search...")
    results.append(bench_full_api_search(ITERATIONS))
    print_result(results[-1])
    
    # Summary
    print_header("RÉSUMÉ")
    print(f"\n{'Layer':<45} {'Mean':>10} {'P95':>10} {'StdDev':>10}")
    print("-" * 75)
    for r in results:
        print(f"{r.name:<45} {r.mean:>8.2f}ms {r.p95:>8.2f}ms {r.stdev:>8.2f}ms")
    
    print_header("FIN DES BENCHMARKS")


if __name__ == "__main__":
    main()
