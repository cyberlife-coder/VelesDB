"""A numpy vector is read through its view, whatever its memory layout."""

import shutil
import tempfile

import numpy as np
import pytest

from velesdb import Database


@pytest.fixture()
def docs():
    path = tempfile.mkdtemp()
    try:
        db = Database(path)
        yield db.create_collection("docs", dimension=4)
    finally:
        shutil.rmtree(path, ignore_errors=True)


@pytest.mark.parametrize("dtype", [np.float32, np.float64])
def test_a_strided_numpy_vector_is_accepted(docs, dtype):
    """A column of a matrix is not contiguous; it used to raise TypeError
    (numpy's AsSliceError) instead of being read."""
    matrix = np.eye(4, dtype=dtype)
    column = matrix[:, 0]
    assert not column.flags["C_CONTIGUOUS"]

    docs.upsert([{"id": 1, "vector": column, "payload": {"k": "v"}}])
    hits = docs.search(vector=column, top_k=1)
    assert [h["id"] for h in hits] == [1]


def test_batch_search_accepts_a_2d_numpy_array(docs):
    """``batch_search`` took ``queries[0]`` as a truth value test on the
    array itself, raising "truth value of an array is ambiguous" where
    ``multi_query_search`` and ``search_batch_parallel`` read the matrix."""
    docs.upsert(
        [
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]},
            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0]},
        ]
    )
    queries = np.eye(4, dtype=np.float32)[:2]
    results = docs.batch_search(queries, top_k=1)
    assert [[h["id"] for h in hits] for hits in results] == [[1], [2]]
    assert docs.batch_search(np.empty((0, 4), dtype=np.float32)) == []


def test_upsert_bulk_numpy_converts_float64(docs):
    """The docs said float64 "forces a conversion"; it raised TypeError."""
    vectors = np.array([[1.0, 0.0, 0.0, 0.0], [0.0, 0.5, 0.0, 0.0]], dtype=np.float64)
    assert docs.upsert_bulk_numpy(vectors, [1, 2], [{"k": 1}, None]) == 2
    assert docs.get([2])[0]["vector"].tolist() == [0.0, 0.5, 0.0, 0.0]
    assert docs.get([1])[0]["payload"] == {"k": 1}


@pytest.mark.parametrize("value", [np.nan, np.inf, 1e39])
def test_upsert_bulk_numpy_refuses_a_float64_that_is_not_a_finite_float32(docs, value):
    vectors = np.array([[1.0, 0.0, 0.0, value]], dtype=np.float64)
    with pytest.raises(ValueError, match="non-finite"):
        docs.upsert_bulk_numpy(vectors, [1])
    assert docs.count() == 0


def test_upsert_bulk_numpy_still_refuses_other_dtypes(docs):
    with pytest.raises(TypeError):
        docs.upsert_bulk_numpy(np.ones((1, 4), dtype=np.int64), [1])
    assert docs.count() == 0
