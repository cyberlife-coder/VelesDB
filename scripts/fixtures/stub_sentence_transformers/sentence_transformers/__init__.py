"""Deterministic stand-in for `sentence_transformers.SentenceTransformer`.

`scripts/run-python-examples.py` puts this package first on `PYTHONPATH` so
the examples that embed text run offline, without downloading a model. Each
text maps to a fixed unit vector seeded by its SHA-256: the examples' calls
into velesdb are exercised for real, their rankings are not meaningful.
"""

import hashlib

import numpy as np

_DIMENSION = 384


class SentenceTransformer:
    def __init__(self, _model_name, *_args, **_kwargs):
        self._dimension = _DIMENSION

    def get_sentence_embedding_dimension(self):
        return self._dimension

    def _embed(self, text):
        seed = int.from_bytes(hashlib.sha256(str(text).encode()).digest()[:8], "little")
        vector = np.random.default_rng(seed).standard_normal(self._dimension)
        return (vector / np.linalg.norm(vector)).astype("float32")

    def encode(self, texts, *_args, **_kwargs):
        if isinstance(texts, str):
            return self._embed(texts)
        return np.stack([self._embed(text) for text in texts])
