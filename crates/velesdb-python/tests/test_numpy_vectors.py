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
