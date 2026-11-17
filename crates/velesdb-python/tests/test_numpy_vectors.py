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
