"""Tests for scripts/check-doc-freshness.py.

Every guard in the freshness checker is pinned here RED-first: each test
builds a synthetic repository that violates exactly one rule, asserts the
guard fails on it, then repairs the violation and asserts the guard passes.
A guard that cannot be shown failing is a guard that protects nothing, which
is precisely how the three release-surface gates in the Premium repo ended up
scanning stub files and passing vacuously.

The checker takes a ``--root`` so none of this touches the real tree.
"""

from __future__ import annotations

import importlib.util
import shutil
import sys
import tempfile
import types
import unittest
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "check-doc-freshness.py"


def _load_script() -> types.ModuleType:
    spec = importlib.util.spec_from_file_location("check_doc_freshness", SCRIPT_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {SCRIPT_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


cdf = _load_script()

WORKSPACE_VERSION = "4.0.0"
MEMORY_VERSION = "0.11.1"

INDEX_TEMPLATE = """# Docs index

- [Alpha](./ALPHA.md)
{extra}
Last updated: 2026-07-25
"""

ALPHA_CLEAN = f"""# Alpha

```toml
velesdb-core = "4.0"
velesdb-memory = {{ version = "{MEMORY_VERSION}" }}
```

Tag: `velesdb-memory-v{MEMORY_VERSION}`

Applies to: velesdb-core {WORKSPACE_VERSION}

Last updated: 2026-07-25
"""


class FreshnessGuardTestCase(unittest.TestCase):
    """Builds a minimal, self-consistent fake repository under a temp dir."""

    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="doc-freshness-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        (self.tmp / "docs").mkdir()
        (self.tmp / "crates" / "velesdb-memory").mkdir(parents=True)
        self.write("Cargo.toml", f'[workspace.package]\nversion = "{WORKSPACE_VERSION}"\n')
        self.write(
            "crates/velesdb-memory/Cargo.toml",
            f'[package]\nname = "velesdb-memory"\nversion = "{MEMORY_VERSION}"\n',
        )
        self.write("docs/README.md", INDEX_TEMPLATE.format(extra=""))
        self.write("docs/ALPHA.md", ALPHA_CLEAN)

    def write(self, rel_path: str, content: str) -> None:
        path = self.tmp / rel_path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")

    def assertGuardPasses(self, guard: str) -> None:
        failures, _info = cdf.GUARDS[guard][0](self.tmp)
        self.assertEqual(failures, [], f"guard '{guard}' should pass but reported {failures}")

    def assertGuardFails(self, guard: str, *expected_substrings: str) -> "list[str]":
        failures, _info = cdf.GUARDS[guard][0](self.tmp)
        self.assertTrue(failures, f"guard '{guard}' should have failed but reported nothing")
        joined = "\n".join(failures)
        for needle in expected_substrings:
            self.assertIn(needle, joined)
        return failures


class StampGuardTests(FreshnessGuardTestCase):
    def test_baseline_repository_passes(self) -> None:
        self.assertGuardPasses("stamp")

    def test_root_doc_without_stamp_fails_then_passes_once_stamped(self) -> None:
        self.write("docs/BETA.md", "# Beta\n\nNo stamp at all.\n")
        self.assertGuardFails("stamp", "docs/BETA.md", "no date stamp")

        self.write("docs/BETA.md", "# Beta\n\nLast updated: 2026-07-25\n")
        self.assertGuardPasses("stamp")

    def test_impossible_calendar_date_is_rejected(self) -> None:
        self.write("docs/BETA.md", "# Beta\n\nLast updated: 2026-02-31\n")
        self.assertGuardFails("stamp", "not a real calendar date")

        self.write("docs/BETA.md", "# Beta\n\nLast updated: 2026-02-28\n")
        self.assertGuardPasses("stamp")

    def test_bold_blockquote_and_inline_stamp_forms_are_accepted(self) -> None:
        # Real forms in use: `> **Last Updated**: ...` (SOUNDNESS.md),
        # `*Last updated: ...*` (BENCHMARKS.md) and a mid-line stamp
        # (`**Version**: 3.10.0 | Last updated: ...` in VELESQL_SPEC.md).
        for body in (
            "> **Last Updated**: 2026-06-12 (full audit)\n",
            "*Last updated: 2026-07-25 - re-measured*\n",
            "**Version**: 3.10.0 | Last updated: 2026-07-25\n",
        ):
            with self.subTest(body=body):
                self.write("docs/BETA.md", f"# Beta\n\n{body}")
                self.assertGuardPasses("stamp")

    def test_docs_readme_itself_is_not_required_to_be_checked_twice(self) -> None:
        self.assertNotIn(
            self.tmp / "docs" / "README.md", cdf.root_docs(self.tmp)
        )


class IndexGuardTests(FreshnessGuardTestCase):
    def test_baseline_repository_passes(self) -> None:
        self.assertGuardPasses("index")

    def test_unlinked_root_doc_fails_then_passes_once_linked(self) -> None:
        self.write("docs/BETA.md", "# Beta\n\nLast updated: 2026-07-25\n")
        self.assertGuardFails("index", "docs/BETA.md", "not linked from docs/README.md")

        self.write("docs/README.md", INDEX_TEMPLATE.format(extra="- [Beta](./BETA.md)\n"))
        self.assertGuardPasses("index")

    def test_same_named_doc_in_a_subdirectory_does_not_satisfy_the_guard(self) -> None:
        # docs/README.md links ./reference/ARCHITECTURE.md while docs/ARCHITECTURE.md
        # also exists: a substring match would call that "linked". It is not.
        self.write("docs/BETA.md", "# Beta\n\nLast updated: 2026-07-25\n")
        self.write("docs/reference/BETA.md", "# Beta (reference)\n")
        self.write(
            "docs/README.md", INDEX_TEMPLATE.format(extra="- [Beta](./reference/BETA.md)\n")
        )
        self.assertGuardFails("index", "docs/BETA.md")

        self.write(
            "docs/README.md",
            INDEX_TEMPLATE.format(extra="- [Beta](./reference/BETA.md)\n- [Beta](./BETA.md)\n"),
        )
        self.assertGuardPasses("index")

    def test_link_with_anchor_or_table_row_form_counts(self) -> None:
        self.write("docs/BETA.md", "# Beta\n\nLast updated: 2026-07-25\n")
        self.write(
            "docs/README.md",
            INDEX_TEMPLATE.format(extra="| [Beta](./BETA.md#usage) | description |\n"),
        )
        self.assertGuardPasses("index")

    def test_reference_style_link_definition_counts(self) -> None:
        self.write("docs/BETA.md", "# Beta\n\nLast updated: 2026-07-25\n")
        self.write(
            "docs/README.md",
            INDEX_TEMPLATE.format(extra="See [beta].\n\n[beta]: ./BETA.md\n"),
        )
        self.assertGuardPasses("index")


DECISIONS_INDEX = """# Decisions

One decision per file. Back to the [docs index](../README.md).

| Decision | Summary |
|----------|---------|
{extra}"""


class DecisionsGuardTests(FreshnessGuardTestCase):
    """docs/decisions/ is swept by nothing else.

    `index` only reaches the root of docs/, and it only asks doc-to-index —
    never index-to-doc, so a row pointing at nothing has always been legal.
    """

    STORAGE = "| [Storage engine](./storage-engine.md) | why the LSM tree |\n"

    def setUp(self) -> None:
        super().setUp()
        self.write("docs/decisions/README.md", DECISIONS_INDEX.format(extra=self.STORAGE))
        self.write("docs/decisions/storage-engine.md", "# Storage engine\n\nStatus: accepted\n")

    def index_with(self, extra: str) -> None:
        self.write("docs/decisions/README.md", DECISIONS_INDEX.format(extra=self.STORAGE + extra))

    def test_baseline_repository_passes(self) -> None:
        self.assertGuardPasses("decisions")

    def test_unlisted_decision_fails_then_passes_once_listed(self) -> None:
        self.write("docs/decisions/wal-fsync.md", "# WAL fsync\n\nStatus: accepted\n")
        self.assertGuardFails(
            "decisions", "docs/decisions/wal-fsync.md", "not listed in docs/decisions/README.md"
        )

        self.index_with("| [WAL fsync](./wal-fsync.md) | why fsync on commit |\n")
        self.assertGuardPasses("decisions")

    def test_dead_link_fails_then_passes_once_the_file_exists(self) -> None:
        self.index_with("| [Tiered cache](./tiered-cache.md) | why the second tier |\n")
        self.assertGuardFails("decisions", "docs/decisions/README.md:", "does not exist")

        self.write("docs/decisions/tiered-cache.md", "# Tiered cache\n\nStatus: accepted\n")
        self.assertGuardPasses("decisions")

    def test_the_index_is_not_required_to_list_itself(self) -> None:
        self.assertNotIn(
            self.tmp / "docs" / "decisions" / "README.md", cdf.decision_docs(self.tmp)
        )

    def test_a_decision_in_a_subdirectory_does_not_satisfy_the_guard(self) -> None:
        # The row points at an archived namesake. A substring match would call
        # docs/decisions/wal-fsync.md "listed". It is not.
        self.write("docs/decisions/wal-fsync.md", "# WAL fsync\n\nStatus: accepted\n")
        self.write("docs/decisions/archive/wal-fsync.md", "# WAL fsync (superseded)\n")
        self.index_with("| [WAL fsync](./archive/wal-fsync.md) | superseded |\n")
        self.assertGuardFails("decisions", "docs/decisions/wal-fsync.md")

    def test_the_back_link_is_checked_but_a_link_leaving_the_tree_is_not(self) -> None:
        # `../README.md` resolves inside the tree and must be verified; a link
        # climbing out of the repository is nobody's business here.
        self.write(
            "docs/decisions/README.md",
            DECISIONS_INDEX.format(extra=self.STORAGE)
            + "\nSee [elsewhere](../../../outside.md).\n",
        )
        self.assertGuardPasses("decisions")

    def test_reference_style_link_definition_counts(self) -> None:
        self.write("docs/decisions/wal-fsync.md", "# WAL fsync\n\nStatus: accepted\n")
        self.write(
            "docs/decisions/README.md",
            DECISIONS_INDEX.format(extra=self.STORAGE) + "\nSee [wal].\n\n[wal]: ./wal-fsync.md\n",
        )
        self.assertGuardPasses("decisions")

    def test_a_missing_decisions_directory_is_a_precondition_not_a_pass(self) -> None:
        # Otherwise `rm -rf docs/decisions/` disarms the guard in silence: CI
        # green, the registry still announcing the subguard, the workflow step
        # still running and measuring nothing. `warn` must not soften it either
        # — a precondition is not a finding.
        shutil.rmtree(self.tmp / "docs" / "decisions")
        for mode in ("strict", "warn"):
            with self.subTest(mode=mode):
                self.assertEqual(
                    cdf.main(["--root", str(self.tmp), "--guard", "decisions", "--mode", mode]),
                    2,
                )


class VersionGuardTests(FreshnessGuardTestCase):
    def test_baseline_repository_passes(self) -> None:
        self.assertGuardPasses("versions")

    def test_stale_applies_to_stamp_fails_then_passes(self) -> None:
        self.write("docs/ALPHA.md", ALPHA_CLEAN.replace(
            f"Applies to: velesdb-core {WORKSPACE_VERSION}", "Applies to: velesdb-core 3.12.0"
        ))
        self.assertGuardFails("versions", "[applies-to-core]", "says 3.12.0", "is 4.0.0")

        self.write("docs/ALPHA.md", ALPHA_CLEAN)
        self.assertGuardPasses("versions")

    def test_stale_cargo_pin_fails_then_passes(self) -> None:
        self.write("docs/ALPHA.md", ALPHA_CLEAN.replace(
            'velesdb-core = "4.0"', 'velesdb-core = "3.2"'
        ))
        self.assertGuardFails("versions", "[cargo-pin-core]", "says 3.2")

        self.write("docs/ALPHA.md", ALPHA_CLEAN)
        self.assertGuardPasses("versions")

    def test_stale_memory_tag_fails_then_passes(self) -> None:
        self.write("docs/ALPHA.md", ALPHA_CLEAN.replace(
            f"velesdb-memory-v{MEMORY_VERSION}", "velesdb-memory-v0.11.0"
        ))
        self.assertGuardFails("versions", "[memory-git-tag]", "says 0.11.0")

        self.write("docs/ALPHA.md", ALPHA_CLEAN)
        self.assertGuardPasses("versions")

    def test_memory_drift_is_measured_against_the_memory_crate_not_the_workspace(self) -> None:
        # velesdb-memory ships on its own version line (0.11.1 while the
        # workspace is 4.0.0). Checking it against the workspace would flag
        # every correct reference.
        self.assertGuardPasses("versions")
        self.write("docs/ALPHA.md", ALPHA_CLEAN.replace(
            f'velesdb-memory = {{ version = "{MEMORY_VERSION}" }}',
            f'velesdb-memory = {{ version = "{WORKSPACE_VERSION}" }}',
        ))
        self.assertGuardFails("versions", "[cargo-pin-memory]", "velesdb-memory is 0.11.1")

    def test_shorter_but_compatible_pin_is_accepted(self) -> None:
        for pin in ('"4"', '"4.0"', '"4.0.0"', '"^4.0"', '"~4.0.0"'):
            with self.subTest(pin=pin):
                self.write("docs/ALPHA.md", ALPHA_CLEAN.replace('"4.0"', pin, 1))
                self.assertGuardPasses("versions")

    def test_longer_or_contradicting_pin_is_rejected(self) -> None:
        for pin in ('"4.1"', '"4.0.1"', '"3"'):
            with self.subTest(pin=pin):
                self.write("docs/ALPHA.md", ALPHA_CLEAN.replace('"4.0"', pin, 1))
                self.assertGuardFails("versions", "[cargo-pin-core]")

    def test_archived_and_migration_docs_are_exempt(self) -> None:
        stale = '```toml\nvelesdb-core = "1.7"\n```\n'
        self.write("docs/archive/OLD_NOTES.md", stale)
        self.write("docs/guides/MIGRATION_v1.7.md", stale)
        self.write("docs/CHANGELOG_EXCERPT.md", stale + "\nLast updated: 2026-07-25\n")
        self.assertGuardPasses("versions")

        # ...but a normal guide with the same content is not exempt.
        self.write("docs/guides/TUNING.md", stale)
        self.assertGuardFails("versions", "docs/guides/TUNING.md")

    def test_stale_published_package_version_in_prose_is_refused(self) -> None:
        self.write(
            "docs/ALPHA.md",
            ALPHA_CLEAN + "\nTimed against the published v3.12.0 packages.\n",
        )
        self.assertGuardFails(
            "versions", "[published-packages-core]", "says 3.12.0"
        )

        self.write(
            "docs/ALPHA.md",
            ALPHA_CLEAN + "\nTested against the published v4.0.0 packages.\n",
        )
        self.assertGuardPasses("versions")

    def test_unqualified_old_product_version_is_refused(self) -> None:
        self.write("docs/ALPHA.md", ALPHA_CLEAN + "\nThis guide targets VelesDB v3.12.0.\n")
        self.assertGuardFails("versions", "[prose-core-version]", "says 3.12.0")

    def test_measurement_word_alone_does_not_exempt_an_old_version(self) -> None:
        self.write(
            "docs/ALPHA.md",
            ALPHA_CLEAN + "\nTimed with VelesDB v3.12.0 on a four-core runner.\n",
        )
        self.assertGuardFails("versions", "[prose-core-version]", "says 3.12.0")

    def test_explicit_historical_product_versions_are_retained(self) -> None:
        historical = (
            "\nFeature introduced in VelesDB v3.12.0.\n"
            "Frozen reference run measured on velesdb-core@1.14.2.\n"
        )
        self.write("docs/ALPHA.md", ALPHA_CLEAN + historical)
        self.assertGuardPasses("versions")

    def test_a_wrapped_sentence_keeps_its_historical_qualifier(self) -> None:
        # `since` and the version it frames land on different lines. A
        # line-scoped window reads the second line alone and calls the
        # reference stale; the qualifier governs the sentence, not the line.
        wrapped = (
            "\nThe scheme is the whole switch: `https://` reaches a hosted\n"
            "provider (supported since\n"
            "velesdb-core 3.12.0) with nothing to rebuild.\n"
        )
        self.write("docs/ALPHA.md", ALPHA_CLEAN + wrapped)
        self.assertGuardPasses("versions")

    def test_a_qualifier_in_a_neighbouring_paragraph_does_not_carry_over(
        self,
    ) -> None:
        # Widening the window to the sentence must not widen it to the page.
        leaky = (
            "\nTLS support was introduced a while ago.\n"
            "\nInstall velesdb-core 3.12.0 for this guide.\n"
        )
        self.write("docs/ALPHA.md", ALPHA_CLEAN + leaky)
        self.assertGuardFails("versions", "[prose-core-version]", "says 3.12.0")

    def test_memory_prose_uses_the_independent_memory_version(self) -> None:
        self.write(
            "docs/ALPHA.md",
            ALPHA_CLEAN + "\nInstall velesdb-memory 0.11.0 for this guide.\n",
        )
        self.assertGuardFails(
            "versions", "[prose-memory-version]", "velesdb-memory is 0.11.1"
        )

    def test_frozen_timing_report_keeps_its_measured_package_versions(self) -> None:
        self.write(
            "docs/quickstart/timing-results.md",
            "Measured with velesdb-core@1.14.2 and published v1.14.2 packages.\n",
        )
        self.assertGuardPasses("versions")


class ModeTests(FreshnessGuardTestCase):
    """`--mode warn` must report the very same problems without failing."""

    def _run(self, mode: str) -> int:
        return cdf.main(["--root", str(self.tmp), "--guard", "index", "--mode", mode])

    def test_strict_exits_1_and_warn_exits_0_on_identical_input(self) -> None:
        self.write("docs/BETA.md", "# Beta\n\nLast updated: 2026-07-25\n")
        self.assertEqual(self._run("strict"), 1)
        self.assertEqual(self._run("warn"), 0)

    def test_strict_exits_0_once_the_problem_is_fixed(self) -> None:
        self.write("docs/BETA.md", "# Beta\n\nLast updated: 2026-07-25\n")
        self.assertEqual(self._run("strict"), 1)
        self.write("docs/README.md", INDEX_TEMPLATE.format(extra="- [Beta](./BETA.md)\n"))
        self.assertEqual(self._run("strict"), 0)


class ManifestReaderTests(FreshnessGuardTestCase):
    def test_workspace_version_is_read_from_the_workspace_package_section(self) -> None:
        # A `[package] version` earlier in the file must not win over the
        # `[workspace.package]` one.
        self.write(
            "Cargo.toml",
            '[package]\nversion = "9.9.9"\n\n[workspace.package]\nversion = "4.0.0"\n',
        )
        self.assertEqual(cdf.read_workspace_version(self.tmp), "4.0.0")

    def test_memory_version_is_read_from_its_own_manifest(self) -> None:
        self.assertEqual(cdf.read_memory_version(self.tmp), MEMORY_VERSION)


if __name__ == "__main__":
    unittest.main()
