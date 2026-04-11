"""BDD tests for Wave 3 Commit 10 typed options dataclasses.

Covers:
- HnswOptions construction + for_dataset_size classmethod
- LimitsOptions round-trip through Database(path, config=...)
- AutoReindexOptions attachment via Database.create_collection(auto_reindex=...)
- VelesConfigOptions wrapping LimitsOptions
- Negative paths: invalid values, wrong types, missing required kwargs

Test categories follow `.claude/rules/bdd-testing.md`:
- Nominal (≥ 60%): happy-path construction + create_collection flow
- Edge (≈ 20%): None fields, boundary values, defaults pass-through
- Negative (≥ 20%): invalid types, breaking-change guardrails
"""

from __future__ import annotations

import pytest
import tempfile
from pathlib import Path

import velesdb
from velesdb import (
    AutoReindexOptions,
    Database,
    HnswOptions,
    LimitsOptions,
    VelesConfigOptions,
)


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
def tmp_db_path():
    with tempfile.TemporaryDirectory() as d:
        yield Path(d) / "db"


# ---------------------------------------------------------------------------
# HnswOptions — nominal
# ---------------------------------------------------------------------------


def test_hnsw_options_default_is_all_none():
    opts = HnswOptions()
    assert opts.m is None
    assert opts.ef_construction is None
    assert opts.max_elements is None
    assert opts.alpha is None
    assert opts.pq_rescore_oversampling is None


def test_hnsw_options_explicit_fields():
    opts = HnswOptions(m=48, ef_construction=600, alpha=1.5)
    assert opts.m == 48
    assert opts.ef_construction == 600
    assert opts.alpha == pytest.approx(1.5)


def test_hnsw_options_for_dataset_size_populates_fields():
    opts = HnswOptions.for_dataset_size(768, 1_000_000)
    assert opts.m is not None and opts.m > 0
    assert opts.ef_construction is not None and opts.ef_construction > 0
    assert opts.max_elements is not None and opts.max_elements > 0


def test_hnsw_options_create_collection_round_trip(tmp_db_path):
    db = Database(str(tmp_db_path))
    col = db.create_collection(
        "docs", dimension=4, hnsw=HnswOptions(m=16, ef_construction=200)
    )
    assert col is not None


# ---------------------------------------------------------------------------
# LimitsOptions — nominal + edge
# ---------------------------------------------------------------------------


def test_limits_options_default_is_all_none():
    opts = LimitsOptions()
    assert opts.max_collections is None
    assert opts.max_dimensions is None


def test_limits_options_explicit_max_collections_enforced_via_database(tmp_db_path):
    """Round-trip: LimitsOptions(max_collections=2) must be honored by Database.
    Creating a 3rd collection on a db opened with this cap must raise."""
    cfg = VelesConfigOptions(limits=LimitsOptions(max_collections=2))
    db = Database(str(tmp_db_path), config=cfg)
    db.create_collection("a", dimension=4)
    db.create_collection("b", dimension=4)
    with pytest.raises(Exception):
        db.create_collection("c", dimension=4)


def test_limits_options_max_dimensions_rejects_oversize(tmp_db_path):
    cfg = VelesConfigOptions(limits=LimitsOptions(max_dimensions=8))
    db = Database(str(tmp_db_path), config=cfg)
    db.create_collection("ok", dimension=4)
    with pytest.raises(Exception):
        db.create_collection("toobig", dimension=16)


# ---------------------------------------------------------------------------
# AutoReindexOptions — nominal + edge
# ---------------------------------------------------------------------------


def test_auto_reindex_options_default_has_engine_defaults():
    opts = AutoReindexOptions()
    assert opts.enabled is True
    assert opts.min_size_for_reindex == 10_000
    assert opts.cooldown_secs == 3_600


def test_auto_reindex_options_disabled_staticmethod():
    opts = AutoReindexOptions.disabled()
    assert opts.enabled is False


def test_auto_reindex_options_explicit_all_fields():
    opts = AutoReindexOptions(
        enabled=True,
        param_divergence_threshold=2.0,
        min_size_for_reindex=500,
        max_latency_regression_percent=5.0,
        max_recall_regression_percent=1.0,
        cooldown_secs=60,
    )
    assert opts.param_divergence_threshold == pytest.approx(2.0)
    assert opts.min_size_for_reindex == 500
    assert opts.cooldown_secs == 60


def test_create_collection_with_auto_reindex_does_not_raise(tmp_db_path):
    db = Database(str(tmp_db_path))
    col = db.create_collection(
        "docs",
        dimension=4,
        auto_reindex=AutoReindexOptions(min_size_for_reindex=1),
    )
    assert col is not None


# ---------------------------------------------------------------------------
# VelesConfigOptions — nominal + edge
# ---------------------------------------------------------------------------


def test_veles_config_options_default_is_empty():
    cfg = VelesConfigOptions()
    assert cfg.limits is None


def test_veles_config_options_with_limits():
    cfg = VelesConfigOptions(limits=LimitsOptions(max_dimensions=512))
    assert cfg.limits is not None
    assert cfg.limits.max_dimensions == 512


def test_database_accepts_none_config(tmp_db_path):
    """Passing config=None is equivalent to not passing it at all."""
    db = Database(str(tmp_db_path), config=None)
    db.create_collection("ok", dimension=4)


# ---------------------------------------------------------------------------
# Breaking-change guardrails — Negative
# ---------------------------------------------------------------------------


def test_create_collection_rejects_removed_m_kwarg(tmp_db_path):
    """v1.12 kwargs `m=` / `ef_construction=` / `expected_vectors=` are
    removed in v1.13 — callers must use `hnsw=HnswOptions(...)` instead."""
    db = Database(str(tmp_db_path))
    with pytest.raises(TypeError):
        db.create_collection("docs", dimension=4, m=48)  # type: ignore[call-arg]


def test_create_collection_rejects_removed_ef_construction_kwarg(tmp_db_path):
    db = Database(str(tmp_db_path))
    with pytest.raises(TypeError):
        db.create_collection("docs", dimension=4, ef_construction=200)  # type: ignore[call-arg]


def test_create_collection_rejects_removed_expected_vectors_kwarg(tmp_db_path):
    db = Database(str(tmp_db_path))
    with pytest.raises(TypeError):
        db.create_collection("docs", dimension=4, expected_vectors=1_000)  # type: ignore[call-arg]


def test_hnsw_options_is_exported_from_top_level():
    """The module __all__ must surface the new dataclasses."""
    assert "HnswOptions" in velesdb.__all__
    assert "LimitsOptions" in velesdb.__all__
    assert "AutoReindexOptions" in velesdb.__all__
    assert "VelesConfigOptions" in velesdb.__all__


# ---------------------------------------------------------------------------
# HnswOptions presets (Wave 3 Commit 11)
# ---------------------------------------------------------------------------


def test_preset_fast_matches_core_fast_params():
    """`HnswOptions.fast()` must match the core `HnswParams::fast()` values
    (M=16, ef_construction=150)."""
    opts = HnswOptions.fast()
    assert opts.m == 16
    assert opts.ef_construction == 150


def test_preset_turbo_matches_core_turbo_params():
    """`HnswOptions.turbo()` must match the core `HnswParams::turbo()` values
    (M=12, ef_construction=100)."""
    opts = HnswOptions.turbo()
    assert opts.m == 12
    assert opts.ef_construction == 100


def test_preset_balanced_small_dim_uses_low_band():
    """`HnswOptions.balanced(128)` must use the low-dim band of
    `HnswParams::auto` (M=24, ef_construction=300)."""
    opts = HnswOptions.balanced(128)
    assert opts.m == 24
    assert opts.ef_construction == 300


def test_preset_balanced_large_dim_uses_high_band():
    """`HnswOptions.balanced(768)` must use the high-dim band of
    `HnswParams::auto` (M=32, ef_construction=400)."""
    opts = HnswOptions.balanced(768)
    assert opts.m == 32
    assert opts.ef_construction == 400


def test_preset_high_recall_bumps_balanced():
    """`high_recall(d)` must exceed `balanced(d)` on both M and ef."""
    balanced = HnswOptions.balanced(768)
    high = HnswOptions.high_recall(768)
    assert high.m > balanced.m
    assert high.ef_construction > balanced.ef_construction


def test_preset_max_recall_does_not_panic():
    """Regression: `HnswOptions.max_recall(dim)` must produce non-None
    fields for every dimension band in the core dispatch."""
    for dim in (64, 128, 256, 512, 768, 1024, 3072):
        opts = HnswOptions.max_recall(dim)
        assert opts.m is not None and opts.m > 0
        assert opts.ef_construction is not None and opts.ef_construction > 0


def test_preset_round_trips_through_create_collection(tmp_db_path):
    """End-to-end: every preset must be accepted by `create_collection`."""
    db = Database(str(tmp_db_path))
    db.create_collection("a", dimension=768, hnsw=HnswOptions.fast())
    db.create_collection("b", dimension=768, hnsw=HnswOptions.turbo())
    db.create_collection("c", dimension=768, hnsw=HnswOptions.balanced(768))
    db.create_collection("d", dimension=768, hnsw=HnswOptions.high_recall(768))
    db.create_collection("e", dimension=768, hnsw=HnswOptions.max_recall(768))
    assert set(db.list_collections()) == {"a", "b", "c", "d", "e"}
