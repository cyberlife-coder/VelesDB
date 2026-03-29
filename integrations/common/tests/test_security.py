import pytest
from velesdb_common.security import (
    validate_batch_size,
    validate_weight,
    validate_storage_mode,
    SecurityError,
    validate_k,
    validate_text,
    ALLOWED_METRICS,
    ALLOWED_STORAGE_MODES,
    STORAGE_MODE_ALIASES,
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


@pytest.mark.parametrize("mode", ["full", "sq8", "binary", "pq", "rabitq"])
def test_validate_storage_mode_accepts_all_modes(mode):
    assert validate_storage_mode(mode) == mode


@pytest.mark.parametrize("mode", ["FULL", "SQ8", "Binary", "PQ", "RaBitQ"])
def test_validate_storage_mode_accepts_case_insensitive(mode):
    assert validate_storage_mode(mode) == mode.lower()


@pytest.mark.parametrize("mode", ["invalid", "fp16", "int4", "", "sq4", "rabit"])
def test_validate_storage_mode_rejects_invalid(mode):
    with pytest.raises(SecurityError, match="Invalid storage mode"):
        validate_storage_mode(mode)


def test_validate_storage_mode_rejects_non_string():
    with pytest.raises(SecurityError, match="must be a string"):
        validate_storage_mode(42)


# ---------------------------------------------------------------------------
# ALLOWED_STORAGE_MODES exhaustive constant check
# ---------------------------------------------------------------------------

def test_allowed_storage_modes_exact_set():
    """ALLOWED_STORAGE_MODES must match the canonical set exactly."""
    if ALLOWED_STORAGE_MODES != frozenset({"full", "sq8", "binary", "pq", "rabitq"}):
        raise AssertionError(
            f"ALLOWED_STORAGE_MODES mismatch: {ALLOWED_STORAGE_MODES!r}"
        )


# ---------------------------------------------------------------------------
# Type errors — non-string inputs
# ---------------------------------------------------------------------------

@pytest.mark.parametrize("bad_value", [None, True, False, [], {}])
def test_validate_storage_mode_rejects_non_string_types(bad_value):
    with pytest.raises(SecurityError, match="must be a string"):
        validate_storage_mode(bad_value)


# ---------------------------------------------------------------------------
# Whitespace — no implicit strip; treated as invalid mode
# ---------------------------------------------------------------------------

@pytest.mark.parametrize("mode", [" pq ", "rabitq ", " full"])
def test_validate_storage_mode_rejects_whitespace_padded(mode):
    with pytest.raises(SecurityError, match="Invalid storage mode"):
        validate_storage_mode(mode)


# ---------------------------------------------------------------------------
# Null bytes — rejected as invalid mode
# ---------------------------------------------------------------------------

def test_validate_storage_mode_rejects_null_bytes():
    with pytest.raises(SecurityError, match="Invalid storage mode"):
        validate_storage_mode("pq\x00")


# ---------------------------------------------------------------------------
# Alias resolution — canonical name must be returned
# ---------------------------------------------------------------------------

@pytest.mark.parametrize("alias,expected_canonical", [
    ("f32", "full"),
    ("int8", "sq8"),
    ("bit", "binary"),
    ("product_quantization", "pq"),
    ("product-quantization", "pq"),
])
def test_validate_storage_mode_resolves_alias(alias, expected_canonical):
    result = validate_storage_mode(alias)
    if result != expected_canonical:
        raise AssertionError(
            f"alias '{alias}' → '{result}', expected '{expected_canonical}'"
        )


# ---------------------------------------------------------------------------
# Alias resolution is case-insensitive
# ---------------------------------------------------------------------------

@pytest.mark.parametrize("alias,expected_canonical", [
    ("F32", "full"),
    ("INT8", "sq8"),
    ("BIT", "binary"),
    ("Product_Quantization", "pq"),
    ("PRODUCT-QUANTIZATION", "pq"),
])
def test_validate_storage_mode_resolves_alias_case_insensitive(alias, expected_canonical):
    result = validate_storage_mode(alias)
    if result != expected_canonical:
        raise AssertionError(
            f"alias '{alias}' → '{result}', expected '{expected_canonical}'"
        )


# ---------------------------------------------------------------------------
# Near-miss aliases — must be rejected (not silently accepted)
# ---------------------------------------------------------------------------

@pytest.mark.parametrize("near_miss", ["f16", "int16", "bits"])
def test_validate_storage_mode_rejects_near_miss_aliases(near_miss):
    with pytest.raises(SecurityError, match="Invalid storage mode"):
        validate_storage_mode(near_miss)


# ---------------------------------------------------------------------------
# STORAGE_MODE_ALIASES constant check
# ---------------------------------------------------------------------------

def test_storage_mode_aliases_maps_all_expected_keys():
    """STORAGE_MODE_ALIASES must cover every alias the Rust core accepts."""
    expected_aliases = {"f32", "int8", "bit", "product_quantization", "product-quantization"}
    missing = expected_aliases - set(STORAGE_MODE_ALIASES.keys())
    extra = set(STORAGE_MODE_ALIASES.keys()) - expected_aliases
    if missing or extra:
        raise AssertionError(
            f"STORAGE_MODE_ALIASES mismatch — missing: {missing}, extra: {extra}"
        )


def test_storage_mode_aliases_all_values_are_canonical():
    """Every value in STORAGE_MODE_ALIASES must be a member of ALLOWED_STORAGE_MODES."""
    invalid = {
        alias: canonical
        for alias, canonical in STORAGE_MODE_ALIASES.items()
        if canonical not in ALLOWED_STORAGE_MODES
    }
    if invalid:
        raise AssertionError(
            f"Aliases point to non-canonical names: {invalid}"
        )
