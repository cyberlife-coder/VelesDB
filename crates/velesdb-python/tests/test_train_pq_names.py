"""
Regression tests for `Database.train_pq` collection-name handling.

`train_pq` used to render VelesQL as text —
``format!("TRAIN QUANTIZER ON {collection_name} WITH (m={m}, k={k}")`` — and
parse it back. Interpolating a name into a query string forced the method to
invent its own identifier rule, and that rule became a second definition of
what a collection may be called. It drifted from `velesdb_core::validation`
in three ways:

- it rejected the interior hyphen core accepts (`validate_collection_name`
  pins `a-b`, and the doc example is `docs-v2`);
- it accepted a leading digit that the VelesQL grammar's `regular_identifier`
  then refused, surfacing as "Failed to construct TRAIN query";
- it accepted the empty string vacuously, rendering
  ``TRAIN QUANTIZER ON  WITH (...)``.

The binding now builds the statement AST directly, as velesdb-mobile and
tauri-plugin-velesdb already do, so no identifier text is produced and no
charset rule is needed.

Run with: pytest tests/test_train_pq_names.py -v
"""

import pytest

from conftest import _SKIP_NO_BINDINGS

# temp_db fixture is provided by conftest.py auto-discovery.
pytestmark = _SKIP_NO_BINDINGS

DIM = 8
POINTS = 256


def _seed(db, name):
    """Creates `name` and fills it with enough vectors to train on."""
    coll = db.create_collection(name, DIM, "cosine")
    coll.upsert(
        [
            {
                "id": i,
                "vector": [((i * 37 + d * 113) % 199) / 199.0 for d in range(DIM)],
            }
            for i in range(POINTS)
        ]
    )
    return coll


@pytest.mark.parametrize("name", ["docs-v2", "a-b-c"])
def test_train_pq_accepts_hyphenated_names_like_core(temp_db, name):
    """A name core accepts must not be rejected by the binding.

    `validate_collection_name` allows interior hyphens and `docs-v2` is its
    own documented example, so a collection created under that name must be
    trainable through the same SDK that created it.
    """
    _seed(temp_db, name)

    result = temp_db.train_pq(name, m=2, k=4)

    assert isinstance(result, str)
    assert "complete" in result.lower()


def test_train_pq_accepts_a_leading_digit(temp_db):
    """A leading digit passed the old charset guard but broke the grammar.

    `regular_identifier` is `(ASCII_ALPHA | "_") ~ (ASCII_ALPHANUMERIC | "_")*`,
    so `2docs` could never be spelled as a bare identifier — the old code
    waved it through its own guard and then failed at parse time with a
    confusing "Failed to construct TRAIN query".
    """
    _seed(temp_db, "2docs")

    result = temp_db.train_pq("2docs", m=2, k=4)

    assert "complete" in result.lower()


def test_train_pq_underscore_names_still_work(temp_db):
    """The names the old guard did allow keep working."""
    _seed(temp_db, "plain_name")

    result = temp_db.train_pq("plain_name", m=2, k=4)

    assert "complete" in result.lower()


def test_train_pq_on_a_missing_collection_raises(temp_db):
    """An unknown name fails in core, with core's error.

    The binding no longer has a private charset guard to fail in first, so
    the error the caller sees is the engine's own.
    """
    with pytest.raises(Exception) as excinfo:
        temp_db.train_pq("never-created", m=2, k=4)

    assert "collection" in str(excinfo.value).lower()


def test_train_pq_on_an_empty_name_raises(temp_db):
    """The empty string satisfied `.all()` vacuously and rendered a
    malformed query; it must be refused."""
    with pytest.raises(Exception):
        temp_db.train_pq("", m=2, k=4)
