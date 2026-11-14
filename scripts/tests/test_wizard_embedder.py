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

    def test_a_recorded_model_is_enforced_even_on_an_empty_store(self):
        # daemon_startup checks the record BEFORE opening: emptiness does not waive it
        self.assertEqual(self.verdict(make_store(self.root / "s", 384, "hash", 0), "all-minilm", 384), emb.MIGRATE)

    def test_an_unprobed_model_on_a_legacy_store_is_unknown(self):
        self.assertEqual(self.verdict(make_store(self.root / "s", 384, None, 3), "my-embed", None), emb.UNKNOWN)

    def test_legacy_store_same_dimension_is_unverified_not_refused(self):
        self.assertEqual(self.verdict(make_store(self.root / "s", 1024, None, 5), "bge-m3", 1024), emb.UNVERIFIED)

    def test_proposals_put_compatible_models_first(self):
        profile = emb.read_store(make_store(self.root / "s", 1024, "bge-m3", 5))
        ranked = emb.rank_models(profile, ["mxbai-embed-large:latest"])
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
        self.assertEqual(emb.server_env(emb.HASH), {"VELESDB_MEMORY_EMBEDDER": "hash"})


    def test_a_versioned_local_model_keeps_its_tag(self):
        ranked = emb.rank_models(emb.StoreProfile(exists=False), ["snowflake-arctic-embed:335m"])
        self.assertIn("snowflake-arctic-embed:335m", [c.name for c in ranked])
        self.assertTrue(emb.is_installed("snowflake-arctic-embed:335m", ["snowflake-arctic-embed:335m"]))
        self.assertFalse(emb.is_installed("snowflake-arctic-embed", ["snowflake-arctic-embed:335m"]))
        self.assertTrue(emb.is_installed("bge-m3", ["bge-m3:latest"]))

    def test_local_embedding_models_outside_the_catalog_are_offered_chat_models_are_not(self):
        ranked = emb.rank_models(emb.StoreProfile(exists=False), ["llama3.2:3b", "snowflake-arctic-embed:latest"])
        names = [c.name for c in ranked]
        self.assertIn("snowflake-arctic-embed", names)
        self.assertNotIn("llama3.2", names)
        self.assertIsNone(next(c for c in ranked if c.name == "snowflake-arctic-embed").dimension)


class SelectionTests(unittest.TestCase):
    def test_the_default_is_the_model_the_store_records(self):
        self.assertEqual(emb.default_embedder(emb.StoreProfile(True, 1024, "bge-m3", True)),
                         emb.Embedder("ollama", "bge-m3", 1024))
        self.assertEqual(emb.default_embedder(emb.StoreProfile(True, 384, "hash", True)), emb.HASH)
        self.assertEqual(emb.default_embedder(emb.StoreProfile(False)), emb.HASH)

    def test_flags_override_the_default(self):
        recorded = emb.StoreProfile(True, 1024, "bge-m3", True)
        self.assertEqual(emb.select_embedder("hash", None, recorded), emb.HASH)
        self.assertEqual(emb.select_embedder("ollama", None, recorded), emb.Embedder("ollama", "bge-m3", 1024))
        self.assertEqual(emb.select_embedder("ollama", None, emb.StoreProfile(False)),
                         emb.Embedder("ollama", "all-minilm", 384))
        self.assertEqual(emb.select_embedder(None, "custom-embed", emb.StoreProfile(False)),
                         emb.Embedder("ollama", "custom-embed", None))

    def test_the_menu_keeps_the_default_on_enter_and_maps_numbers(self):
        store = emb.StoreProfile(False)
        answers = iter(["", "0", "x", "1"])
        ask = lambda _prompt: next(answers)
        quiet = lambda _line: None
        self.assertEqual(emb.choose_embedder(store, [], ask, quiet), emb.HASH)
        self.assertEqual(emb.choose_embedder(store, [], ask, quiet), emb.HASH)
        self.assertEqual(emb.choose_embedder(store, [], ask, quiet).model, emb.rank_models(store, [])[0].name)


class WiringTests(unittest.TestCase):
    def test_daemon_flags_are_always_explicit(self):
        bge = emb.Embedder("ollama", "bge-m3", 1024)
        self.assertEqual(emb.daemon_flags(bge, "linux"), ["--embedder=ollama", "--ollama-model=bge-m3"])
        self.assertEqual(emb.daemon_flags(bge, "windows"), ["-Embedder", "ollama", "-OllamaModel", "bge-m3"])
        self.assertEqual(emb.daemon_flags(emb.HASH, "macos"), ["--embedder=hash"])

    def test_migration_stages_next_to_the_store(self):
        cmd = emb.migration_cmd("/bin/vm", Path("/home/u/.velesdb-memory"), stamp="20261114T120000Z")
        self.assertEqual(cmd, ["/bin/vm", "migrate-embeddings", "--store", "/home/u/.velesdb-memory",
                               "--destination", "/home/u/.velesdb-memory.migration-20261114T120000Z"])

    def test_each_migration_gets_its_own_destination(self):
        # a completed migration leaves a committed journal beside its destination,
        # and migrate-embeddings refuses to rerun there (switchover.rs entry_state)
        store = Path("/home/u/.velesdb-memory")
        first = emb.migration_cmd("/bin/vm", store, stamp="20261114T120000Z")[-1]
        second = emb.migration_cmd("/bin/vm", store, stamp="20261115T090000Z")[-1]
        self.assertNotEqual(first, second)

    def test_a_custom_endpoint_reaches_the_server_and_the_daemon(self):
        bge = emb.Embedder("ollama", "bge-m3", 1024, url="http://127.0.0.1:11435")
        self.assertEqual(emb.server_env(bge)["VELESDB_MEMORY_EMBEDDER_URL"], "http://127.0.0.1:11435")
        self.assertIn("--ollama-url=http://127.0.0.1:11435", emb.daemon_flags(bge, "linux"))
        self.assertEqual(emb.daemon_flags(bge, "windows")[-2:], ["-OllamaUrl", "http://127.0.0.1:11435"])

    def test_the_endpoint_is_the_one_the_server_would_read(self):
        self.assertEqual(emb.resolve_url("http://h:1/", {}), "http://h:1")
        self.assertEqual(emb.resolve_url(None, {"VELESDB_MEMORY_OLLAMA_URL": "http://h:2"}), "http://h:2")
        self.assertEqual(emb.resolve_url(None, {"VELESDB_MEMORY_EMBEDDER_URL": "http://h:3",
                                                "VELESDB_MEMORY_OLLAMA_URL": "http://h:2"}), "http://h:3")
        self.assertIsNone(emb.resolve_url(None, {}))
        self.assertIsNone(emb.resolve_url("http://localhost:11434", {}))

    def test_an_endpoint_that_is_not_http_is_refused(self):
        # The URL is fetched by the wizard and written into every harness
        # config and the daemon: only an http(s) endpoint with a host is one.
        for bad in ("file:///etc/passwd", "ftp://h:1", "localhost:11434", "http://", "javascript:alert(1)"):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                emb.resolve_url(bad, {})
        with self.assertRaises(ValueError):
            emb.resolve_url(None, {"VELESDB_MEMORY_OLLAMA_URL": "file:///tmp/x"})
        self.assertEqual(emb.resolve_url("https://ollama.lan:443", {}), "https://ollama.lan:443")

    def test_owned_keys_cover_aliases_and_leave_the_users_variables(self):
        owned = emb.owned_env_keys(emb.server_env(emb.Embedder("ollama", "bge-m3", 1024, url="http://h:1")))
        self.assertTrue({"VELESDB_MEMORY_OLLAMA_MODEL", "VELESDB_MEMORY_OLLAMA_URL"} <= owned)
        self.assertNotIn("VELESDB_MEMORY_EXTRACTOR", owned)
        self.assertNotIn("VELESDB_MEMORY_OLLAMA_URL", emb.owned_env_keys(emb.server_env(emb.HASH)))

    def test_probed_dimension_reads_the_first_vector(self):
        self.assertEqual(emb.probed_dimension({"embeddings": [[0.1] * 768]}), 768)
        self.assertIsNone(emb.probed_dimension({"error": "not an embedding model"}))
        self.assertIsNone(emb.probed_dimension(None))


class FakeOps:
    """Records commands; Ollama answers once `up` is true."""

    def __init__(self, up=True, models=(), dims=None, commands=(), rc=0, installer=None):
        self.up, self.models, self.dims, self.installer = up, list(models), dims or {}, installer
        self.commands, self.rc, self.ran = set(commands), rc, []
        self.url = "http://localhost:11434"

    def has(self, command):
        return command in self.commands

    def run(self, cmd, env=None):
        self.ran.append((cmd, env))
        if cmd[:2] == ["ollama", "pull"] and self.rc == 0:
            self.models.append(cmd[2] + ":latest")
        if cmd[:2] == ["brew", "install"] and self.rc == 0:
            self.up = True
        return self.rc

    def tags(self):
        return list(self.models) if self.up else None

    def embed_dimension(self, model):
        return self.dims.get(model)

    def fetch_installer(self):
        return self.installer


def session(ops, answer=True, yes=False):
    return emb.Session(ops, confirm=lambda _q: answer, yes=yes, say=lambda _l: None)


class OllamaStepTests(unittest.TestCase):
    def test_a_missing_model_is_pulled_then_its_real_dimension_probed(self):
        ops = FakeOps(dims={"bge-m3": 1024})
        ready, statuses = emb.prepare_ollama(emb.Embedder("ollama", "bge-m3", 1024), "linux", session(ops))
        self.assertEqual(ready, emb.Embedder("ollama", "bge-m3", 1024))
        self.assertIn((["ollama", "pull", "bge-m3"], None), ops.ran)
        self.assertIn("1024d", statuses[-1])

    def test_the_probe_wins_over_the_catalog(self):
        ops = FakeOps(models=["nomic-embed-text:latest"], dims={"nomic-embed-text": 512})
        ready, statuses = emb.prepare_ollama(emb.Embedder("ollama", "nomic-embed-text", 768), "linux", session(ops))
        self.assertEqual(ready.dimension, 512)
        self.assertIn("not the catalog's 768d", statuses[-1])

    def test_declining_the_pull_refuses(self):
        ready, statuses = emb.prepare_ollama(emb.Embedder("ollama", "bge-m3", 1024), "linux",
                                             session(FakeOps(), answer=False))
        self.assertIsNone(ready)
        self.assertIn("REFUSED", statuses[-1])

    def test_ollama_is_installed_with_consent_through_the_package_manager(self):
        ops = FakeOps(up=False, dims={"all-minilm": 384}, commands={"brew"})
        ready, statuses = emb.prepare_ollama(emb.Embedder("ollama", "all-minilm", 384), "macos", session(ops))
        self.assertEqual(ops.ran[0], (["brew", "install", "ollama"], None))
        self.assertEqual(ready.dimension, 384)
        self.assertEqual(statuses[0], "ollama: installed")

    def test_an_installed_but_stopped_ollama_is_reported_not_reinstalled(self):
        ops = FakeOps(up=False, commands={"ollama", "brew"})
        ready, statuses = emb.prepare_ollama(emb.Embedder("ollama", "all-minilm", 384), "macos", session(ops))
        self.assertIsNone(ready)
        self.assertEqual(ops.ran, [])
        self.assertIn("ollama serve", statuses[0])

    def _downloaded_installer(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        script = Path(tmp.name) / "ollama-install.sh"
        script.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        return script

    def test_yes_never_runs_a_downloaded_installer(self):
        # --yes accepts what a package manager verifies, never a script
        # fetched from the network: that one needs a person's explicit answer.
        script = self._downloaded_installer()
        ops = FakeOps(up=False, installer=script)
        ready, statuses = emb.prepare_ollama(emb.Embedder("ollama", "all-minilm", 384), "linux",
                                             session(ops, answer=False, yes=True))
        self.assertIsNone(ready)
        self.assertEqual(ops.ran, [])
        self.assertIn("REFUSED", statuses[0])
        self.assertIn(emb.LINUX_INSTALL_SCRIPT, statuses[0])
        self.assertFalse(script.exists())

    def test_the_downloaded_installer_runs_on_an_explicit_answer_then_is_removed(self):
        script = self._downloaded_installer()
        ops = FakeOps(up=False, installer=script)
        emb.prepare_ollama(emb.Embedder("ollama", "all-minilm", 384), "linux", session(ops, answer=True))
        self.assertEqual(ops.ran[0], (["sh", str(script)], None))
        self.assertFalse(script.exists())

    def test_hash_needs_no_ollama(self):
        ops = FakeOps(up=False)
        self.assertEqual(emb.prepare_ollama(emb.HASH, "linux", session(ops)), (emb.HASH, []))


class ReconcileTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)

    def test_a_compatible_store_is_wired_without_migration(self):
        ops = FakeOps()
        ok, _ = emb.reconcile_store(emb.Embedder("ollama", "bge-m3", 1024),
                                    make_store(self.root / "s", 1024, "bge-m3", 4), "/bin/vm", session(ops), False)
        self.assertTrue(ok)
        self.assertEqual(ops.ran, [])

    def test_an_incompatible_store_is_migrated_with_the_target_embedder_env(self):
        ops = FakeOps()
        store = make_store(self.root / "s", 384, "hash", 4)
        ok, statuses = emb.reconcile_store(emb.Embedder("ollama", "bge-m3", 1024), store, "/bin/vm",
                                           session(ops), False)
        self.assertTrue(ok, statuses)
        cmd, env = ops.ran[0]
        self.assertEqual(cmd[:2], ["/bin/vm", "migrate-embeddings"])
        self.assertEqual(env, {"VELESDB_MEMORY_EMBEDDER": "ollama", "VELESDB_MEMORY_EMBEDDER_MODEL": "bge-m3"})

    def test_declining_the_migration_refuses_to_wire(self):
        ok, statuses = emb.reconcile_store(emb.Embedder("ollama", "bge-m3", 1024),
                                           make_store(self.root / "s", 384, "hash", 4), "/bin/vm",
                                           session(FakeOps(), answer=False), False)
        self.assertFalse(ok)
        self.assertIn("REFUSED", statuses[0])

    def test_a_locked_store_is_not_migrated_offline(self):
        ops = FakeOps()
        ok, statuses = emb.reconcile_store(emb.Embedder("ollama", "bge-m3", 1024),
                                           make_store(self.root / "s", 384, "hash", 4), "/bin/vm",
                                           session(ops, yes=True), True)
        self.assertFalse(ok)
        self.assertEqual(ops.ran, [])
        self.assertIn("locked", statuses[0])


if __name__ == "__main__":
    unittest.main()
