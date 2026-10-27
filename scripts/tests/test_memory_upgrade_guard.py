"""The upgrade guard decides, so its decisions are tested without a build.

`scripts/memory_upgrade_guard.py` builds the last published `velesdb-memory`
and this tree into two copies of one driver, then replays what a user does:
write a store with the old crate, open it with the new one, write again, roll
back, and kill the writer mid-write. The builds and the processes cannot run
in a unit test; what can, and what would silently turn the guard into a
no-op if it were wrong, is every decision around them:

  * which published version is the base (the latest usable release);
  * the retrieval floor handed to the new build (relative to the old one);
  * the order of the steps, and which build runs each;
  * that a refusal of the new build fails the guard, and a refusal the guard
    EXPECTS (the second opener of a locked store) does not.
"""

from __future__ import annotations

import contextlib
import json
import unittest
from pathlib import Path

from scripts import memory_upgrade_guard as guard


def index_line(version: str, yanked: bool = False) -> str:
    return json.dumps({"name": "velesdb-memory", "vers": version, "yanked": yanked})


class PublishedBaseTests(unittest.TestCase):
    """The base is what users have installed: the latest usable release."""

    def releases(self, *entries: tuple[str, bool]) -> list[guard.Release]:
        text = "\n".join(index_line(v, y) for v, y in entries)
        return guard.parse_index(text)

    def test_the_highest_release_is_the_base(self) -> None:
        releases = self.releases(("0.13.0", False), ("0.14.1", False), ("0.14.2", False))
        self.assertEqual(guard.pick_base(releases), "0.14.2")

    def test_a_yanked_release_is_never_the_base(self) -> None:
        releases = self.releases(("0.14.1", False), ("0.14.2", True))
        self.assertEqual(guard.pick_base(releases), "0.14.1")

    def test_a_prerelease_is_never_the_base(self) -> None:
        releases = self.releases(("0.14.2", False), ("0.15.0-rc.1", False))
        self.assertEqual(guard.pick_base(releases), "0.14.2")

    def test_versions_compare_as_numbers_not_as_text(self) -> None:
        releases = self.releases(("0.9.0", False), ("0.10.0", False))
        self.assertEqual(guard.pick_base(releases), "0.10.0")

    def test_an_index_without_a_usable_release_is_an_error_not_a_pass(self) -> None:
        with self.assertRaises(guard.GuardError):
            guard.pick_base(self.releases(("0.14.2", True)))

    def test_blank_lines_in_the_index_are_ignored(self) -> None:
        releases = guard.parse_index("\n" + index_line("0.14.2") + "\n\n")
        self.assertEqual([r.version for r in releases], ["0.14.2"])


class RetrievalFloorTests(unittest.TestCase):
    """Search over an approximate index may differ; it may not collapse."""

    def test_the_floor_follows_the_previous_version_less_a_margin(self) -> None:
        self.assertAlmostEqual(guard.relative_floor(0.93), 0.90)

    def test_the_floor_never_drops_below_the_absolute_minimum(self) -> None:
        self.assertEqual(guard.relative_floor(0.50), guard.ABSOLUTE_FLOOR)

    def test_the_floor_never_exceeds_one(self) -> None:
        self.assertLessEqual(guard.relative_floor(1.0), 1.0)

    def test_the_rate_is_read_from_the_driver_output(self) -> None:
        out = "INFO  retrieval_rate=0.9318 (2789/2993 facts found)\nOK — 2993 live facts"
        self.assertAlmostEqual(guard.retrieval_rate(out) or 0.0, 0.9318)

    def test_a_driver_that_reports_no_rate_gives_none(self) -> None:
        self.assertIsNone(guard.retrieval_rate("OK — nothing else"))


class ManifestTests(unittest.TestCase):
    def test_the_old_build_pins_the_published_version_exactly(self) -> None:
        manifest = guard.driver_manifest("old", guard.published_dependency("0.14.2"))
        self.assertIn('version = "=0.14.2"', manifest)
        self.assertIn("[workspace]", manifest, "must not join the repository workspace")

    def test_the_new_build_points_at_this_tree(self) -> None:
        manifest = guard.driver_manifest("new", guard.tree_dependency(Path("/repo")))
        self.assertIn('path = "/repo/crates/velesdb-memory"', manifest)

    def test_both_builds_ask_for_the_features_the_driver_uses(self) -> None:
        for dependency in (guard.published_dependency("0.14.2"), guard.tree_dependency(Path("/r"))):
            self.assertIn('"persistence"', dependency)
            self.assertIn('"context"', dependency)
            self.assertIn("default-features = false", dependency)


class PlanTests(unittest.TestCase):
    def steps(self, crashes: int = 0) -> list[guard.Step]:
        return guard.plan(facts=300, crashes=crashes)

    def names(self, crashes: int = 0) -> list[str]:
        return [step.name for step in self.steps(crashes)]

    def test_the_order_is_seed_control_upgrade_write_reopen_rollback_lock(self) -> None:
        self.assertEqual(
            self.names(),
            [
                "seed with the previous version",
                "control: the previous version reads its own store",
                "upgrade: this tree opens the previous version's store",
                "write with this tree",
                "reopen: this tree reads what it wrote",
                "rollback: the previous version opens this tree's store",
                "lock: a second opener is refused",
            ],
        )

    def test_each_crash_round_adds_two_scenarios(self) -> None:
        self.assertEqual(len(self.names(crashes=3)) - len(self.names()), 3 * 2)

    def test_the_crash_scenarios_start_from_a_fresh_store_and_from_the_previous_versions(self) -> None:
        crash = [s for s in self.steps(crashes=1) if s.kind == "crash"]
        self.assertEqual([s.binary for s in crash], ["new", "new"])
        self.assertEqual(sorted(s.source for s in crash), ["", "store-old"])

    def test_the_upgrade_runs_the_new_build_on_a_copy_of_the_old_store(self) -> None:
        step = next(s for s in self.steps() if s.name.startswith("upgrade"))
        self.assertEqual(step.binary, "new")
        self.assertEqual(step.copy_from, "store-old")
        self.assertNotEqual(step.copy_to, step.copy_from, "the original must never be written")

    def test_the_rollback_runs_the_previous_build_on_a_copy_of_the_new_store(self) -> None:
        step = next(s for s in self.steps() if s.name.startswith("rollback"))
        self.assertEqual((step.binary, step.copy_from), ("old", "store-up"))

    def test_the_new_build_is_checked_against_a_floor_and_the_control_records_it(self) -> None:
        by_name = {s.name: s for s in self.steps()}
        self.assertTrue(by_name["control: the previous version reads its own store"].records_rate)
        self.assertTrue(by_name["upgrade: this tree opens the previous version's store"].uses_floor)

    def test_only_the_lock_step_needs_a_process_holding_the_store(self) -> None:
        self.assertEqual([s.kind for s in self.steps() if s.kind == "lock"], ["lock"])


class FakeProcesses:
    """Stands in for the driver binaries, the kill cycle and the lock holder."""

    def __init__(self, outputs: dict[str, tuple[int, str]] | None = None) -> None:
        self.outputs = outputs or {}
        self.calls: list[tuple[str, tuple[str, ...]]] = []
        self.killed: list[tuple[str, str, str]] = []
        self.held: list[tuple[str, str]] = []

    def run(self, binary: str, args: tuple[str, ...]) -> tuple[int, str]:
        self.calls.append((binary, args))
        return self.outputs.get(f"{binary} {args[0]}", (0, "OK"))

    def kill_cycle(self, binary: str, store: str, acks: str) -> None:
        self.killed.append((binary, store, acks))

    @contextlib.contextmanager
    def hold(self, binary: str, store: str):
        self.held.append((binary, store))
        yield


class VerdictTests(unittest.TestCase):
    def run_guard(self, outputs: dict[str, tuple[int, str]] | None = None, crashes: int = 0):
        fake = FakeProcesses(outputs)
        code = guard.execute(
            guard.plan(facts=300, crashes=crashes),
            work=Path("/work"),
            run=fake.run,
            kill_cycle=fake.kill_cycle,
            hold=fake.hold,
            copy_store=lambda src, dst: None,
            report=lambda line: None,
        )
        return code, fake

    def test_a_clean_run_passes(self) -> None:
        code, _ = self.run_guard({"old verify": (0, "INFO  retrieval_rate=0.93 (x)\nOK")})
        self.assertEqual(code, 0)

    def test_a_loss_after_the_upgrade_fails_the_guard(self) -> None:
        code, _ = self.run_guard({"new verify": (1, "LOSS  fact 1 is absent from the export")})
        self.assertEqual(code, 1)

    def test_a_loss_after_the_rollback_fails_the_guard(self) -> None:
        code, _ = self.run_guard({"old verify": (1, "LOSS  fact 1 is absent from the export")})
        self.assertEqual(code, 1)

    def test_a_driver_that_dies_without_a_verdict_fails_the_guard(self) -> None:
        code, _ = self.run_guard({"old seed": (101, "panicked at src/main.rs")})
        self.assertEqual(code, 1)

    def test_a_step_that_follows_a_failed_seed_is_not_run_on_nothing(self) -> None:
        _, fake = self.run_guard({"old seed": (101, "panicked")})
        self.assertEqual([c[1][0] for c in fake.calls], ["seed"])

    def test_a_second_opener_that_is_let_in_fails_the_guard(self) -> None:
        code, fake = self.run_guard({"new probe-lock": (1, "FAILED — two processes opened the same store")})
        self.assertEqual(code, 1)
        self.assertEqual(len(fake.held), 1, "the store must be held while it is probed")

    def test_a_lost_acknowledged_write_after_a_kill_fails_the_guard(self) -> None:
        code, fake = self.run_guard({"new check": (1, "12 acknowledged writes, 1 lost")}, crashes=1)
        self.assertEqual(code, 1)
        self.assertEqual(len(fake.killed), 1, "the guard stops at the first loss")

    def test_the_new_build_is_given_a_floor_derived_from_the_control(self) -> None:
        _, fake = self.run_guard({"old verify": (0, "INFO  retrieval_rate=0.9300 (x)\nOK")})
        upgrade = next(a for b, a in fake.calls if b == "new" and a[0] == "verify")
        self.assertAlmostEqual(float(upgrade[-1]), 0.90, places=2)

    def test_without_a_rate_in_the_control_the_absolute_floor_applies(self) -> None:
        _, fake = self.run_guard({"old verify": (0, "OK — no rate printed")})
        upgrade = next(a for b, a in fake.calls if b == "new" and a[0] == "verify")
        self.assertAlmostEqual(float(upgrade[-1]), guard.ABSOLUTE_FLOOR)


if __name__ == "__main__":
    unittest.main()
