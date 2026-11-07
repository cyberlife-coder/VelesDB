"""Every binding surface's `gpu` opt-in resolves to `velesdb-core/gpu` (#1965).

`velesdb-python` has always exposed `gpu = ["velesdb-core/gpu"]`; a Node user
had no equivalent because `velesdb-node` may never name `velesdb-core` — the
ci.yml "License/dependency boundary gate" (`cargo tree -p velesdb-node
--depth 1`) refuses a direct edge. Its passthrough therefore forwards through
`velesdb-memory`, whose own `gpu` forwards to core.

Three rules hold the chain:

* a surface with a direct `velesdb-core` dependency forwards
  `gpu = ["velesdb-core/gpu"]`;
* `velesdb-node` forwards `gpu = ["velesdb-memory/gpu"]` and names
  `velesdb-core` in no dependency table;
* every non-`default` feature `velesdb-memory` declares is checked in
  isolation by ci.yml's per-feature loop — a feature the loop never names
  is code nobody compiles.
"""

from __future__ import annotations

import re
import tomllib
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent

DIRECT_CORE_SURFACES = [
    "crates/velesdb-memory/Cargo.toml",
    "crates/velesdb-server/Cargo.toml",
    "crates/velesdb-python/Cargo.toml",
    "crates/velesdb-mobile/Cargo.toml",
]
NODE_MANIFEST = "crates/velesdb-node/Cargo.toml"
DEPENDENCY_TABLES = ("dependencies", "dev-dependencies", "build-dependencies")


def load(relative: str) -> dict:
    return tomllib.loads((REPO_ROOT / relative).read_text(encoding="utf-8"))


def gpu_forward(manifest: dict) -> list:
    return manifest.get("features", {}).get("gpu", [])


def declared_dependencies(manifest: dict) -> set:
    deps = set()
    for table in DEPENDENCY_TABLES:
        deps.update(manifest.get(table, {}))
    return deps


def isolation_loop_features(ci_yml: str) -> set:
    match = re.search(r"for feat in ([a-z -]+); do", ci_yml)
    return set(match.group(1).split()) if match else set()


class PureFunctionTests(unittest.TestCase):
    def test_gpu_forward_reads_the_features_table(self) -> None:
        manifest = {"features": {"gpu": ["velesdb-core/gpu"]}}
        self.assertEqual(gpu_forward(manifest), ["velesdb-core/gpu"])

    def test_gpu_forward_absent_is_empty_not_an_error(self) -> None:
        self.assertEqual(gpu_forward({}), [])
        self.assertEqual(gpu_forward({"features": {"persistence": []}}), [])

    def test_declared_dependencies_unions_all_tables(self) -> None:
        manifest = {
            "dependencies": {"a": "1"},
            "dev-dependencies": {"b": "1"},
            "build-dependencies": {"c": "1"},
        }
        self.assertEqual(declared_dependencies(manifest), {"a", "b", "c"})


class RealTreeTests(unittest.TestCase):
    def test_direct_core_surfaces_forward_gpu_to_core(self) -> None:
        for relative in DIRECT_CORE_SURFACES:
            with self.subTest(manifest=relative):
                self.assertEqual(
                    gpu_forward(load(relative)),
                    ["velesdb-core/gpu"],
                    f"{relative} must forward gpu to velesdb-core/gpu exactly",
                )

    def test_node_forwards_gpu_through_memory_only(self) -> None:
        node = load(NODE_MANIFEST)
        self.assertEqual(
            gpu_forward(node),
            ["velesdb-memory/gpu"],
            "velesdb-node's gpu must resolve through velesdb-memory — a "
            "velesdb-core/gpu entry implies a direct dep the boundary gate refuses",
        )

    def test_node_declares_no_velesdb_core_dependency(self) -> None:
        self.assertNotIn(
            "velesdb-core",
            declared_dependencies(load(NODE_MANIFEST)),
            "velesdb-node must never declare velesdb-core (license boundary)",
        )

    def test_ci_checks_each_memory_feature_in_isolation(self) -> None:
        memory_features = set(load("crates/velesdb-memory/Cargo.toml").get("features", {}))
        memory_features.discard("default")
        loop_features = isolation_loop_features(
            (REPO_ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        )
        self.assertTrue(loop_features, "ci.yml's per-feature isolation loop was not found")
        self.assertEqual(
            memory_features - loop_features,
            set(),
            "velesdb-memory features missing from ci.yml's isolated-feature loop",
        )


if __name__ == "__main__":
    unittest.main()
