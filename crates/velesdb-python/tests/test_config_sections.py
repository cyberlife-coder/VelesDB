"""BDD tests for issue #1549 — full engine `VelesConfig` coverage in Python.

`VelesConfigOptions` historically exposed only the `limits` section. This
suite covers the extension to the remaining *engine* sections
(`search`, `hnsw`, `storage`, `quantization`) plus TOML loading via
`VelesConfigOptions.from_toml` / `from_toml_path` (engine-only semantics,
mirroring `VelesConfig::from_toml_engine_only` /
`load_from_path_engine_only` from PR #1565).

Scope guards:
- `wal_batch` stays NOT exposed (velesdb-premium Enterprise feature —
  see docs/guides/WRITE_CONCURRENCY.md).
- `server` / `logging` are out of scope for the embedded surface; a TOML
  shared with a hosting shell must still load (engine-only filtering).

Test categories:
- Nominal (>= 60%): section construction + TOML happy paths + round-trip
  through `Database(path, config=...)`
- Edge (~20%): defaults pass-through, engine-only section filtering
- Negative (>= 20%): invalid TOML, invalid values, wal_batch guardrails
"""

from __future__ import annotations

import tempfile
from pathlib import Path

import pytest

import velesdb
from velesdb import (
    Database,
    HnswConfigOptions,
    LimitsOptions,
    QuantizationOptions,
    SearchConfigOptions,
    StorageOptions,
    VelesConfigOptions,
)


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
def tmp_db_path():
    with tempfile.TemporaryDirectory() as d:
        yield Path(d) / "db"


@pytest.fixture
def tmp_dir():
    with tempfile.TemporaryDirectory() as d:
        yield Path(d)


# ---------------------------------------------------------------------------
# Section dataclasses — nominal construction
# ---------------------------------------------------------------------------


def test_search_config_options_default_is_all_none():
    opts = SearchConfigOptions()
    assert opts.default_mode is None
    assert opts.ef_search is None
    assert opts.max_results is None
    assert opts.query_timeout_ms is None


def test_search_config_options_explicit_fields():
    opts = SearchConfigOptions(
        default_mode="accurate", ef_search=256, max_results=42, query_timeout_ms=5_000
    )
    assert opts.default_mode == "accurate"
    assert opts.ef_search == 256
    assert opts.max_results == 42
    assert opts.query_timeout_ms == 5_000


def test_hnsw_config_options_default_is_all_none():
    opts = HnswConfigOptions()
    assert opts.m is None
    assert opts.ef_construction is None
    assert opts.max_layers is None


def test_hnsw_config_options_explicit_fields():
    opts = HnswConfigOptions(m=32, ef_construction=400, max_layers=8)
    assert opts.m == 32
    assert opts.ef_construction == 400
    assert opts.max_layers == 8


def test_storage_options_default_is_all_none():
    opts = StorageOptions()
    assert opts.data_dir is None
    assert opts.storage_mode is None
    assert opts.mmap_cache_mb is None
    assert opts.vector_alignment is None


def test_storage_options_explicit_fields():
    opts = StorageOptions(
        data_dir="./data", storage_mode="memory", mmap_cache_mb=256, vector_alignment=32
    )
    assert opts.data_dir == "./data"
    assert opts.storage_mode == "memory"
    assert opts.mmap_cache_mb == 256
    assert opts.vector_alignment == 32


def test_quantization_options_default_is_all_none():
    opts = QuantizationOptions()
    assert opts.mode is None
    assert opts.pq_m is None
    assert opts.pq_k is None
    assert opts.pq_opq_enabled is None
    assert opts.pq_oversampling is None
    assert opts.rerank_enabled is None
    assert opts.rerank_multiplier is None
    assert opts.auto_quantization is None
    assert opts.auto_quantization_threshold is None


def test_quantization_options_explicit_sq8():
    opts = QuantizationOptions(mode="sq8", rerank_enabled=False, rerank_multiplier=3)
    assert opts.mode == "sq8"
    assert opts.rerank_enabled is False
    assert opts.rerank_multiplier == 3


def test_quantization_options_explicit_pq():
    opts = QuantizationOptions(mode="pq", pq_m=8, pq_k=128, pq_opq_enabled=True)
    assert opts.mode == "pq"
    assert opts.pq_m == 8
    assert opts.pq_k == 128
    assert opts.pq_opq_enabled is True


# ---------------------------------------------------------------------------
# VelesConfigOptions — new section kwargs
# ---------------------------------------------------------------------------


def test_veles_config_options_accepts_all_engine_sections():
    cfg = VelesConfigOptions(
        limits=LimitsOptions(max_collections=10),
        search=SearchConfigOptions(max_results=100),
        hnsw=HnswConfigOptions(m=16),
        storage=StorageOptions(mmap_cache_mb=128),
        quantization=QuantizationOptions(mode="sq8"),
    )
    assert cfg.limits.max_collections == 10
    assert cfg.search.max_results == 100
    assert cfg.hnsw.m == 16
    assert cfg.storage.mmap_cache_mb == 128
    assert cfg.quantization.mode == "sq8"


def test_veles_config_options_sections_default_to_none():
    cfg = VelesConfigOptions()
    assert cfg.limits is None
    assert cfg.search is None
    assert cfg.hnsw is None
    assert cfg.storage is None
    assert cfg.quantization is None


def test_new_section_classes_are_exported_from_top_level():
    for name in (
        "SearchConfigOptions",
        "HnswConfigOptions",
        "StorageOptions",
        "QuantizationOptions",
    ):
        assert name in velesdb.__all__, f"{name} missing from velesdb.__all__"


# ---------------------------------------------------------------------------
# Sections are actually applied at Database open time
# ---------------------------------------------------------------------------


def test_database_open_applies_search_section(tmp_db_path):
    """An out-of-range `search.max_results` must be rejected at open —
    proof the section reaches the core engine config (not dropped)."""
    cfg = VelesConfigOptions(search=SearchConfigOptions(max_results=0))
    with pytest.raises(ValueError, match="search.max_results"):
        Database(str(tmp_db_path), config=cfg)


def test_database_open_applies_hnsw_section(tmp_db_path):
    cfg = VelesConfigOptions(hnsw=HnswConfigOptions(m=2))  # < 4 → invalid
    with pytest.raises(ValueError, match="hnsw.m"):
        Database(str(tmp_db_path), config=cfg)


def test_database_open_applies_storage_section(tmp_db_path):
    cfg = VelesConfigOptions(storage=StorageOptions(storage_mode="floppy"))
    with pytest.raises(ValueError, match="storage.storage_mode"):
        Database(str(tmp_db_path), config=cfg)


def test_database_open_rejects_invalid_search_mode(tmp_db_path):
    cfg = VelesConfigOptions(search=SearchConfigOptions(default_mode="warp"))
    with pytest.raises(ValueError, match="default_mode"):
        Database(str(tmp_db_path), config=cfg)


def test_database_open_rejects_pq_without_m(tmp_db_path):
    cfg = VelesConfigOptions(quantization=QuantizationOptions(mode="pq"))
    with pytest.raises(ValueError, match="pq_m"):
        Database(str(tmp_db_path), config=cfg)


def test_database_open_rejects_pq_fields_without_pq_mode(tmp_db_path):
    """pq_* fields with a non-pq mode must fail fast, never be silently
    dropped."""
    cfg = VelesConfigOptions(quantization=QuantizationOptions(mode="sq8", pq_m=8))
    with pytest.raises(ValueError, match="pq_"):
        Database(str(tmp_db_path), config=cfg)


def test_database_open_with_valid_full_config_works(tmp_db_path):
    cfg = VelesConfigOptions(
        limits=LimitsOptions(max_collections=3),
        search=SearchConfigOptions(default_mode="fast", max_results=50),
        hnsw=HnswConfigOptions(m=16, ef_construction=200),
        storage=StorageOptions(storage_mode="mmap", mmap_cache_mb=64),
        quantization=QuantizationOptions(mode="none"),
    )
    db = Database(str(tmp_db_path), config=cfg)
    db.create_collection("ok", dimension=4)
    assert db.list_collections() == ["ok"]


# ---------------------------------------------------------------------------
# TOML loading — nominal
# ---------------------------------------------------------------------------

FULL_TOML = """
[search]
default_mode = "accurate"
ef_search = 256
max_results = 42
query_timeout_ms = 5000

[hnsw]
m = 32
ef_construction = 400
max_layers = 8

[storage]
data_dir = "./custom_data"
storage_mode = "memory"
mmap_cache_mb = 256
vector_alignment = 32

[limits]
max_collections = 5
max_dimensions = 512

[quantization]
rerank_enabled = false
rerank_multiplier = 3
auto_quantization = false
auto_quantization_threshold = 50000
"""


def test_from_toml_happy_path_populates_every_section():
    cfg = VelesConfigOptions.from_toml(FULL_TOML)
    assert cfg.search.default_mode == "accurate"
    assert cfg.search.ef_search == 256
    assert cfg.search.max_results == 42
    assert cfg.search.query_timeout_ms == 5000
    assert cfg.hnsw.m == 32
    assert cfg.hnsw.ef_construction == 400
    assert cfg.hnsw.max_layers == 8
    assert cfg.storage.data_dir == "./custom_data"
    assert cfg.storage.storage_mode == "memory"
    assert cfg.storage.mmap_cache_mb == 256
    assert cfg.storage.vector_alignment == 32
    assert cfg.limits.max_collections == 5
    assert cfg.limits.max_dimensions == 512
    assert cfg.quantization.rerank_enabled is False
    assert cfg.quantization.rerank_multiplier == 3
    assert cfg.quantization.auto_quantization is False
    assert cfg.quantization.auto_quantization_threshold == 50000


def test_from_toml_unset_sections_carry_engine_defaults():
    """A TOML with only [limits] still yields fully-populated sections
    (engine defaults), so `to_core` round-trips losslessly."""
    cfg = VelesConfigOptions.from_toml("[limits]\nmax_collections = 7\n")
    assert cfg.limits.max_collections == 7
    # Engine defaults surfaced, not None:
    assert cfg.search.max_results == 1000
    assert cfg.search.default_mode == "balanced"
    assert cfg.storage.storage_mode == "mmap"
    assert cfg.quantization.mode == "none"


def test_from_toml_pq_mode_round_trips():
    cfg = VelesConfigOptions.from_toml(
        '[quantization]\nmode = { type = "pq", m = 8 }\n'
    )
    assert cfg.quantization.mode == "pq"
    assert cfg.quantization.pq_m == 8
    assert cfg.quantization.pq_k == 256  # core default k


def test_from_toml_path_happy_path(tmp_dir):
    path = tmp_dir / "velesdb.toml"
    path.write_text(FULL_TOML)
    cfg = VelesConfigOptions.from_toml_path(str(path))
    assert cfg.limits.max_collections == 5
    assert cfg.search.default_mode == "accurate"


def test_from_toml_config_is_applied_by_database(tmp_db_path):
    """End-to-end: a TOML-limit of 1 collection must be enforced by a
    Database opened with the loaded config."""
    cfg = VelesConfigOptions.from_toml("[limits]\nmax_collections = 1\n")
    db = Database(str(tmp_db_path), config=cfg)
    db.create_collection("a", dimension=4)
    with pytest.raises(Exception):
        db.create_collection("b", dimension=4)


def test_from_toml_ignores_shell_owned_server_section():
    """Engine-only semantics (PR #1565): a shell-owned [server] table with
    a low bind port must not leak into the engine config nor fail
    validation; the genuine engine sections still apply."""
    cfg = VelesConfigOptions.from_toml(
        "[server]\nport = 443\n\n[limits]\nmax_collections = 5\n"
    )
    assert cfg.limits.max_collections == 5


# ---------------------------------------------------------------------------
# TOML loading — negative (fail-fast, typed messages)
# ---------------------------------------------------------------------------


def test_from_toml_invalid_syntax_raises_value_error():
    with pytest.raises(ValueError):
        VelesConfigOptions.from_toml("this is [not valid toml")


def test_from_toml_invalid_value_carries_typed_config_error():
    """`limits.max_collections = 0` must surface the core ConfigError
    message (key + range), never fall back to defaults silently."""
    with pytest.raises(ValueError, match="limits.max_collections"):
        VelesConfigOptions.from_toml("[limits]\nmax_collections = 0\n")


def test_from_toml_path_missing_file_raises_file_not_found():
    with pytest.raises(FileNotFoundError):
        VelesConfigOptions.from_toml_path("/nonexistent/velesdb.toml")


# ---------------------------------------------------------------------------
# wal_batch stays unexposed (velesdb-premium Enterprise feature)
# ---------------------------------------------------------------------------


def test_veles_config_options_rejects_wal_batch_kwarg():
    with pytest.raises(TypeError):
        VelesConfigOptions(wal_batch={"enabled": True})  # type: ignore[call-arg]


def test_veles_config_options_has_no_wal_batch_attribute():
    assert not hasattr(VelesConfigOptions(), "wal_batch")


def test_from_toml_does_not_surface_wal_batch_section():
    """[wal_batch] in a TOML is valid engine input for the server/CLI, but
    the embedded Python surface does not expose it (Enterprise-only —
    docs/guides/WRITE_CONCURRENCY.md)."""
    cfg = VelesConfigOptions.from_toml("[wal_batch]\nenabled = true\n")
    assert not hasattr(cfg, "wal_batch")
