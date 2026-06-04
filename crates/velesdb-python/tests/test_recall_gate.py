"""
Recall quality gate — ensures search accuracy doesn't regress.

Threshold: recall@10 >= 0.95 at 10K vectors for all distance metrics.
See QUALITY_BAR.md (Gate 1) for rationale.

Run with: pytest tests/test_recall_gate.py -v
Run slow tests only: pytest tests/test_recall_gate.py -m slow -v
"""

import shutil
import tempfile

import numpy as np
import pytest

from conftest import _SKIP_NO_BINDINGS, VELESDB_AVAILABLE

pytestmark = _SKIP_NO_BINDINGS

if VELESDB_AVAILABLE:
    from velesdb import Database
else:
    Database = None  # type: ignore[assignment,misc]

N_VECTORS = 10_000
DIM = 128
K = 10
N_QUERIES = 30
RECALL_THRESHOLD = 0.95


@pytest.fixture(scope="module")
def recall_env():
    """Shared vectors and database for all recall tests in this module.

    Module-scoped because inserting 10 K vectors three times (once per metric)
    in separate function-scoped fixtures would make the suite ~3x slower with
    no isolation benefit — every test gets its own named collection anyway.
    """
    np.random.seed(42)
    vectors = np.random.randn(N_VECTORS, DIM).astype(np.float32)
    tmp = tempfile.mkdtemp()
    db = Database(tmp)
    yield db, vectors
    shutil.rmtree(tmp, ignore_errors=True)


def _insert_collection(db, vectors, name, metric):
    """Insert all N_VECTORS into a new collection in 1 000-vector batches.

    Invariant: point id == index into `vectors`.  _measure_recall relies on
    this so that gt_ids returned by the brute-force function (which are numpy
    argsort indices) map directly to the ids stored in the collection.
    """
    col = db.create_collection(name, dimension=DIM, metric=metric)
    for start in range(0, N_VECTORS, 1000):
        end = min(start + 1000, N_VECTORS)
        batch = [
            {"id": i, "vector": vectors[i].tolist(), "payload": {"idx": i}}
            for i in range(start, end)
        ]
        col.upsert(batch)
    return col


def _measure_recall(col, vectors, brute_fn):
    """Measure mean recall@K over N_QUERIES random query vectors."""
    np.random.seed(123)
    query_indices = np.random.choice(N_VECTORS, N_QUERIES, replace=False)
    recalls = []
    for qi in query_indices:
        query = vectors[qi]
        gt_ids = brute_fn(vectors, query, K)
        results = col.search(query.tolist(), top_k=K)
        hnsw_ids = {int(r["id"]) for r in results}
        recalls.append(len(gt_ids & hnsw_ids) / K)
    return sum(recalls) / len(recalls)


def _brute_cosine(vectors, query, k):
    norms = np.linalg.norm(vectors, axis=1, keepdims=True)
    norms[norms == 0] = 1.0
    query_norm = max(float(np.linalg.norm(query)), 1e-10)
    sims = (vectors / norms) @ (query / query_norm)
    return {int(i) for i in np.argsort(-sims)[:k]}


def _brute_euclidean(vectors, query, k):
    dists = np.sum((vectors - query[np.newaxis, :]) ** 2, axis=1)
    return {int(i) for i in np.argsort(dists)[:k]}


def _brute_dot(vectors, query, k):
    dots = vectors @ query
    return {int(i) for i in np.argsort(-dots)[:k]}


@pytest.mark.slow
class TestRecallGate:
    """Recall@10 must remain >= 0.95 for all distance metrics at 10 K vectors."""

    def test_recall_cosine(self, recall_env):
        db, vectors = recall_env
        # Seed 0 → cosine-specific random data for broader coverage
        np.random.seed(0)
        col = _insert_collection(db, vectors, "recall_cos", "cosine")
        recall = _measure_recall(col, vectors, _brute_cosine)
        assert recall >= RECALL_THRESHOLD, (
            f"Cosine recall@{K} = {recall:.4f} < {RECALL_THRESHOLD}"
        )

    def test_recall_euclidean(self, recall_env):
        db, vectors = recall_env
        # Seed 1 → euclidean-specific random data for broader coverage
        np.random.seed(1)
        col = _insert_collection(db, vectors, "recall_euc", "euclidean")
        recall = _measure_recall(col, vectors, _brute_euclidean)
        assert recall >= RECALL_THRESHOLD, (
            f"Euclidean recall@{K} = {recall:.4f} < {RECALL_THRESHOLD}"
        )

    def test_recall_dot(self, recall_env):
        db, vectors = recall_env
        # Seed 2 → dot-specific random data for broader coverage
        np.random.seed(2)
        col = _insert_collection(db, vectors, "recall_dot", "dot")
        recall = _measure_recall(col, vectors, _brute_dot)
        assert recall >= RECALL_THRESHOLD, (
            f"DotProduct recall@{K} = {recall:.4f} < {RECALL_THRESHOLD}"
        )
