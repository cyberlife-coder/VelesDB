"""Tests for the shared fusion strategy builder (`velesdb_common.fusion`).

`resolve_weighted_params` is a pure function with no dependency on the
compiled `velesdb` (Rust/PyO3) extension, so these tests exercise the default
"weighted" fusion weights without needing to build the native bindings.
"""

from velesdb_common.fusion import (
    DEFAULT_WEIGHTED_AVG_WEIGHT,
    DEFAULT_WEIGHTED_HIT_WEIGHT,
    DEFAULT_WEIGHTED_MAX_WEIGHT,
    resolve_weighted_params,
)

# Canonical defaults, single-sourced from `velesdb-core`
# (crates/velesdb-core/src/fusion/strategy.rs:
#  DEFAULT_WEIGHTED_AVG_WEIGHT / DEFAULT_WEIGHTED_MAX_WEIGHT / DEFAULT_WEIGHTED_HIT_WEIGHT).
#
# Duplicated here as plain literals rather than importing the Rust crate from
# Python (issue #1545 explicitly rules out a cross-language import for this).
# If either side's literal is ever edited without the other, this test is the
# tripwire that catches the drift.
CORE_DEFAULT_AVG_WEIGHT = 0.6
CORE_DEFAULT_MAX_WEIGHT = 0.3
CORE_DEFAULT_HIT_WEIGHT = 0.1


def test_default_weighted_weights_match_core_canonical_constants():
    assert DEFAULT_WEIGHTED_AVG_WEIGHT == CORE_DEFAULT_AVG_WEIGHT
    assert DEFAULT_WEIGHTED_MAX_WEIGHT == CORE_DEFAULT_MAX_WEIGHT
    assert DEFAULT_WEIGHTED_HIT_WEIGHT == CORE_DEFAULT_HIT_WEIGHT


def test_default_weighted_weights_sum_to_one():
    total = (
        DEFAULT_WEIGHTED_AVG_WEIGHT
        + DEFAULT_WEIGHTED_MAX_WEIGHT
        + DEFAULT_WEIGHTED_HIT_WEIGHT
    )
    assert abs(total - 1.0) < 1e-9


def test_resolve_weighted_params_none_returns_canonical_defaults():
    assert resolve_weighted_params(None) == (
        DEFAULT_WEIGHTED_AVG_WEIGHT,
        DEFAULT_WEIGHTED_MAX_WEIGHT,
        DEFAULT_WEIGHTED_HIT_WEIGHT,
    )


def test_resolve_weighted_params_empty_dict_returns_canonical_defaults():
    assert resolve_weighted_params({}) == (
        DEFAULT_WEIGHTED_AVG_WEIGHT,
        DEFAULT_WEIGHTED_MAX_WEIGHT,
        DEFAULT_WEIGHTED_HIT_WEIGHT,
    )


def test_resolve_weighted_params_partial_override_keeps_other_defaults():
    avg, max_w, hit = resolve_weighted_params({"avg_weight": 0.9})
    assert avg == 0.9
    assert max_w == DEFAULT_WEIGHTED_MAX_WEIGHT
    assert hit == DEFAULT_WEIGHTED_HIT_WEIGHT


def test_resolve_weighted_params_full_override():
    assert resolve_weighted_params(
        {"avg_weight": 0.5, "max_weight": 0.3, "hit_weight": 0.2}
    ) == (0.5, 0.3, 0.2)
