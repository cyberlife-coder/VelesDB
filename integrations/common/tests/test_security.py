import pytest
from velesdb_common.security import (
    validate_batch_size,
    validate_weight,
    SecurityError,
    validate_k,
    validate_text,
    ALLOWED_METRICS,
    ALLOWED_STORAGE_MODES,
)


def test_validate_batch_size_rejects_negative():
    with pytest.raises(SecurityError, match="non-negative"):
        validate_batch_size(-1)


def test_validate_batch_size_accepts_zero():
    assert validate_batch_size(0) == 0


def test_validate_batch_size_accepts_valid():
    assert validate_batch_size(100) == 100


def test_validate_weight_rejects_bool_true():
    with pytest.raises(SecurityError, match="not bool"):
        validate_weight(True)


def test_validate_weight_rejects_bool_false():
    with pytest.raises(SecurityError, match="not bool"):
        validate_weight(False)


def test_validate_weight_accepts_valid_float():
    assert validate_weight(0.5) == 0.5
    assert validate_weight(0.0) == 0.0
    assert validate_weight(1.0) == 1.0


def test_allowed_metrics_is_frozenset():
    assert isinstance(ALLOWED_METRICS, frozenset)


def test_allowed_storage_modes_is_frozenset():
    assert isinstance(ALLOWED_STORAGE_MODES, frozenset)
