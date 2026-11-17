"""A scipy sparse query reads like the dict query it encodes.

The scipy path densified the matrix, flattened every row into one vector
and dropped every weight at or below ``f32::EPSILON`` — where the dict path
and core keep any non-zero weight.
"""

import shutil
import tempfile

import pytest

scipy_sparse = pytest.importorskip("scipy.sparse")

from conftest import _SKIP_NO_BINDINGS  # noqa: E402

pytestmark = _SKIP_NO_BINDINGS

try:
    from velesdb import Database
except (ImportError, AttributeError):
    Database = None  # type: ignore[assignment,misc]

VOCAB = 16


@pytest.fixture
def collection():
    path = tempfile.mkdtemp()
    try:
        db = Database(path)
        coll = db.create_collection("docs", dimension=2)
        coll.upsert(
            [
                {"id": 1, "vector": [1.0, 0.0], "sparse_vector": {3: 1e-8}},
                {"id": 2, "vector": [0.0, 1.0], "sparse_vector": {5: 0.5}},
            ]
        )
        yield coll
    finally:
        shutil.rmtree(path, ignore_errors=True)


def _ids(hits):
    return [h["id"] for h in hits]


def test_a_small_weight_survives_the_scipy_path(collection):
    query = scipy_sparse.csr_matrix(([1e-8], ([0], [3])), shape=(1, VOCAB))
    assert _ids(collection.search(sparse_vector=query, top_k=5)) == _ids(
        collection.search(sparse_vector={3: 1e-8}, top_k=5)
    ) == [1]


def test_a_multi_row_matrix_is_refused_not_flattened(collection):
    two_rows = scipy_sparse.csr_matrix(([1.0, 1.0], ([0, 1], [3, 5])), shape=(2, VOCAB))
    with pytest.raises(ValueError, match="one row"):
        collection.search(sparse_vector=two_rows, top_k=5)


def test_a_one_row_matrix_reads_its_columns(collection):
    query = scipy_sparse.coo_matrix(([0.5], ([0], [5])), shape=(1, 2**31))
    assert _ids(collection.search(sparse_vector=query, top_k=5)) == [2]
