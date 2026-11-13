"""Tests for scripts/wizard_embedder.py — embedder choice against an existing store."""

from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "wizard_embedder.py"


def _load():
    spec = importlib.util.spec_from_file_location("wizard_embedder", SCRIPT_PATH)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


emb = _load()


def make_store(root: Path, dim: int, model: "str | None", points: int = 0) -> Path:
    """A store laid out as velesdb-memory writes it."""
    for name in ("_semantic_memory", "_episodic_memory", "_procedural_memory"):
        (root / name).mkdir(parents=True)
        (root / name / "config.json").write_text(json.dumps({"name": name, "dimension": dim, "point_count": points}))
    if model is not None:
        (root / "embedding-provenance.json").write_text(json.dumps({"model": model, "dimension": dim}))
    return root


class CatalogTests(unittest.TestCase):
    def test_catalog_names_the_dimension_of_every_proposed_model(self):
        dims = {m.name: m.dimension for m in emb.CATALOG}
        self.assertEqual(dims, {"all-minilm": 384, "nomic-embed-text": 768, "mxbai-embed-large": 1024, "bge-m3": 1024})

    def test_hash_is_the_offline_384_dim_default(self):
        self.assertEqual(emb.HASH_DIMENSION, 384)


class StoreProfileTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)

    def test_absent_store(self):
        profile = emb.read_store(self.root / "nope")
        self.assertFalse(profile.exists)

    def test_recorded_store_with_facts(self):
        profile = emb.read_store(make_store(self.root / "s", 384, "hash", points=12))
        self.assertEqual((profile.exists, profile.dimension, profile.model, profile.has_facts), (True, 384, "hash", True))

    def test_legacy_store_without_provenance(self):
        profile = emb.read_store(make_store(self.root / "s", 1024, None))
        self.assertEqual((profile.dimension, profile.model, profile.has_facts), (1024, None, False))


class CompatibilityTests(unittest.TestCase):
    """VELES-004 (dimension) and provenance (same-dimension, other model)."""

    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)

    def verdict(self, store: Path, model: str, dimension: int) -> str:
        return emb.compatibility(emb.read_store(store), model, dimension)

    def test_no_store_accepts_any_model(self):
        self.assertEqual(self.verdict(self.root / "none", "bge-m3", 1024), emb.FRESH)

    def test_same_model_matches(self):
        self.assertEqual(self.verdict(make_store(self.root / "s", 1024, "bge-m3", 5), "bge-m3", 1024), emb.MATCH)

    def test_same_dimension_other_model_with_facts_needs_migration(self):
        # hash and all-minilm are both 384-dim: core opens it, recall is noise
        self.assertEqual(self.verdict(make_store(self.root / "s", 384, "hash", 5), "all-minilm", 384), emb.MIGRATE)

    def test_other_dimension_needs_migration_even_when_empty(self):
        self.assertEqual(self.verdict(make_store(self.root / "s", 384, "hash", 0), "bge-m3", 1024), emb.MIGRATE)

    def test_empty_store_of_the_same_dimension_takes_a_new_model(self):
        self.assertEqual(self.verdict(make_store(self.root / "s", 384, "hash", 0), "all-minilm", 384), emb.MATCH)

    def test_legacy_store_same_dimension_is_unverified_not_refused(self):
        self.assertEqual(self.verdict(make_store(self.root / "s", 1024, None, 5), "bge-m3", 1024), emb.UNVERIFIED)

    def test_proposals_put_compatible_models_first(self):
        profile = emb.read_store(make_store(self.root / "s", 1024, "bge-m3", 5))
        ranked = emb.rank_models(profile, local_models=["mxbai-embed-large:latest"])
        self.assertEqual(ranked[0].name, "bge-m3")
        self.assertEqual(ranked[0].verdict, emb.MATCH)
        self.assertTrue(all(r.verdict == emb.MIGRATE for r in ranked[1:]))
        self.assertTrue(next(r for r in ranked if r.name == "mxbai-embed-large").installed)


class OllamaTests(unittest.TestCase):
    def test_local_embedding_models_are_listed_from_api_tags(self):
        tags = {"models": [{"name": "bge-m3:latest"}, {"name": "llama3.2:3b"}, {"name": "my-embed:latest"}]}
        self.assertEqual(emb.local_models(tags), ["bge-m3:latest", "llama3.2:3b", "my-embed:latest"])

    def test_install_command_per_os_is_explicit(self):
        self.assertEqual(emb.ollama_install_command("macos", has=lambda c: c == "brew"), ["brew", "install", "ollama"])
        self.assertEqual(emb.ollama_install_command("windows", has=lambda c: c == "winget"),
                         ["winget", "install", "--id", "Ollama.Ollama", "-e", "--accept-source-agreements",
                          "--accept-package-agreements"])
        self.assertIsNone(emb.ollama_install_command("macos", has=lambda c: False))

    def test_harness_env_names_ollama_and_omits_the_default(self):
        self.assertEqual(emb.server_env(emb.Embedder("ollama", "bge-m3", 1024)),
                         {"VELESDB_MEMORY_EMBEDDER": "ollama", "VELESDB_MEMORY_EMBEDDER_MODEL": "bge-m3"})
        self.assertEqual(emb.server_env(emb.HASH), {})


if __name__ == "__main__":
    unittest.main()
