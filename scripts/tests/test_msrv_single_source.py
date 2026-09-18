"""`rust-toolchain.toml`'s channel is the MSRV crates.io consumers read.

Three rules hold that:

* the workspace `rust-version`, which crates.io consumers read, equals the
  toolchain file's `channel`;
* a member crate inherits that field (`rust-version.workspace = true`) or
  leaves it out, and never declares its own;
* `.clippy.toml`'s `msrv`, which clippy reads to decide which lints apply,
  equals the workspace `rust-version`. Nothing else reads that key, so
  without this rule it drifts in silence.

What a workflow may install is not checked here: that rule, with every form
of pin it refuses, belongs to `test_ci_toolchain_pin.py` alone.
"""

from __future__ import annotations

import tomllib
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent


def declares_its_own_rust_version(manifest: dict) -> bool:
    return isinstance(manifest.get("package", {}).get("rust-version"), str)


def load(relative: str) -> dict:
    return tomllib.loads((REPO_ROOT / relative).read_text(encoding="utf-8"))


class MemberTests(unittest.TestCase):
    def test_a_declared_rust_version_is_its_own(self) -> None:
        self.assertTrue(declares_its_own_rust_version({"package": {"rust-version": "1.85"}}))

    def test_inheriting_or_omitting_the_field_is_not(self) -> None:
        self.assertFalse(declares_its_own_rust_version({"package": {"rust-version": {"workspace": True}}}))
        self.assertFalse(declares_its_own_rust_version({"package": {}}))


class RealTreeTests(unittest.TestCase):
    def test_the_toolchain_file_is_the_workspace_msrv(self) -> None:
        channel = load("rust-toolchain.toml")["toolchain"]["channel"]
        msrv = load("Cargo.toml")["workspace"]["package"]["rust-version"]
        self.assertEqual(channel, msrv, "rust-toolchain.toml and the workspace rust-version name different compilers")

    def test_no_member_declares_its_own_rust_version(self) -> None:
        members = load("Cargo.toml")["workspace"]["members"]
        self.assertTrue(members, "the workspace lists no member: this test would check nothing")
        own = [m for m in members if declares_its_own_rust_version(load(f"{m}/Cargo.toml"))]
        self.assertEqual(own, [], "these members declare a rust-version instead of inheriting the workspace's")

    def test_the_clippy_msrv_is_the_workspace_msrv(self) -> None:
        clippy_msrv = load(".clippy.toml")["msrv"]
        msrv = load("Cargo.toml")["workspace"]["package"]["rust-version"]
        self.assertEqual(clippy_msrv, msrv, ".clippy.toml's msrv and the workspace rust-version name different compilers")


if __name__ == "__main__":
    unittest.main()
