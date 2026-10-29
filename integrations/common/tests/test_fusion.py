"""Tests for the shared fusion strategy builder (`velesdb_common.fusion`).

`resolve_weighted_params` is a pure function with no dependency on the
compiled `velesdb` (Rust/PyO3) extension, so these tests exercise the default
"weighted" fusion weights without needing to build the native bindings.
"""

import sys
import types

import pytest

from velesdb_common.fusion import (
    DEFAULT_WEIGHTED_AVG_WEIGHT,
    DEFAULT_WEIGHTED_HIT_WEIGHT,
    DEFAULT_WEIGHTED_MAX_WEIGHT,
    build_fusion_strategy,
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


class _RecordingStrategy:
    """Stand-in for ``velesdb.FusionStrategy`` that records how it was built."""

    calls = []

    @staticmethod
    def from_name(name, **params):
        _RecordingStrategy.calls.append((name, params))
        return ("from_name", name, params)


def _fake_velesdb(monkeypatch, strategy_cls):
    module = types.ModuleType("velesdb")
    module.FusionStrategy = strategy_cls
    monkeypatch.setitem(sys.modules, "velesdb", module)


def test_the_name_is_read_by_cores_grammar_when_the_binding_has_it(monkeypatch):
    _RecordingStrategy.calls = []
    _fake_velesdb(monkeypatch, _RecordingStrategy)

    built = build_fusion_strategy("AVG", {"k": 30, "dense_weight": 0.7, "unrelated": 1})

    assert built == ("from_name", "AVG", {"k": 30, "dense_weight": 0.7})
    assert _RecordingStrategy.calls == [("AVG", {"k": 30, "dense_weight": 0.7})]


def test_an_older_binding_falls_back_to_the_literal_dispatch(monkeypatch):
    class _OldStrategy:
        @staticmethod
        def weighted(avg_weight, max_weight, hit_weight):
            return ("weighted", avg_weight, max_weight, hit_weight)

    _fake_velesdb(monkeypatch, _OldStrategy)

    assert build_fusion_strategy("weighted") == (
        "weighted",
        DEFAULT_WEIGHTED_AVG_WEIGHT,
        DEFAULT_WEIGHTED_MAX_WEIGHT,
        DEFAULT_WEIGHTED_HIT_WEIGHT,
    )
    with pytest.raises(ValueError):
        build_fusion_strategy("nope")


def _binding_has_from_name():
    try:
        import velesdb
    except ImportError:
        return False
    return hasattr(velesdb.FusionStrategy, "from_name")


@pytest.mark.skipif(
    not _binding_has_from_name(),
    reason="installed velesdb predates velesdb.FusionStrategy.from_name",
)
def test_the_real_binding_accepts_cores_aliases_and_refuses_unknown_names():
    import velesdb

    assert isinstance(build_fusion_strategy("Max"), velesdb.FusionStrategy)
    assert isinstance(build_fusion_strategy("rsf", {"dense_weight": 0.8}), velesdb.FusionStrategy)
    with pytest.raises(ValueError):
        build_fusion_strategy("nope")
