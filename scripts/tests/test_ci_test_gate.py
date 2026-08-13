"""Contract tests for the process wrappers used by the Node and WASM jobs."""

from __future__ import annotations

import subprocess
import contextlib
import io
import unittest
from pathlib import Path
from unittest import mock

from scripts import ci_test_runner


class RunStagesTests(unittest.TestCase):
    def test_every_stage_runs_in_order(self) -> None:
        stages = (
            ci_test_runner.Stage("install", ("npm", "install")),
            ci_test_runner.Stage("build", ("npx", "napi", "build")),
            ci_test_runner.Stage("test", ("npm", "test")),
        )
        completed = subprocess.CompletedProcess([], 0)

        with mock.patch.object(
            ci_test_runner.subprocess, "run", return_value=completed
        ) as run, contextlib.redirect_stdout(io.StringIO()):
            result = ci_test_runner.run_stages(stages, Path("/fixture"))

        self.assertEqual(result, 0)
        self.assertEqual(
            [call.args[0] for call in run.call_args_list],
            [list(stage.command) for stage in stages],
        )
        self.assertTrue(all(call.kwargs["cwd"] == Path("/fixture") for call in run.call_args_list))

    def test_a_failed_stage_refuses_and_stops_the_pipeline(self) -> None:
        stages = (
            ci_test_runner.Stage("install", ("npm", "install")),
            ci_test_runner.Stage("build", ("npx", "napi", "build")),
            ci_test_runner.Stage("test", ("npm", "test")),
        )
        results = (
            subprocess.CompletedProcess([], 0),
            subprocess.CompletedProcess([], 9),
        )

        with mock.patch.object(
            ci_test_runner.subprocess, "run", side_effect=results
        ) as run, contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(
            io.StringIO()
        ):
            result = ci_test_runner.run_stages(stages, Path("/fixture"))

        self.assertEqual(result, 1)
        self.assertEqual(run.call_count, 2, "nothing may run after the refusing stage")

    def test_an_unstartable_tool_is_an_operational_error(self) -> None:
        stages = (ci_test_runner.Stage("test", ("missing-tool", "test")),)
        with mock.patch.object(
            ci_test_runner.subprocess, "run", side_effect=FileNotFoundError("missing")
        ), contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(
            io.StringIO()
        ):
            self.assertEqual(ci_test_runner.run_stages(stages, Path("/fixture")), 2)


class CommandContractTests(unittest.TestCase):
    def test_linux_node_gate_keeps_the_workflow_sequence(self) -> None:
        self.assertEqual(
            ci_test_runner.node_stages("npm-x", "npx-x"),
            (
                ci_test_runner.Stage(
                    "Install Node dependencies",
                    ("npm-x", "install", "--no-audit", "--no-fund"),
                ),
                ci_test_runner.Stage(
                    "Build the napi addon",
                    ("npx-x", "napi", "build", "--platform"),
                ),
                ci_test_runner.Stage("Run the Node suites", ("npm-x", "test")),
            ),
        )

    def test_windows_node_gate_keeps_the_single_portable_spec(self) -> None:
        self.assertEqual(
            ci_test_runner.windows_node_stages("npm-x", "npx-x", "node-x")[-1],
            ci_test_runner.Stage(
                "Run the Windows Node suite",
                ("node-x", "--test", "__test__/index.spec.mjs"),
            ),
        )

    def test_wasm_gate_keeps_the_node_runner(self) -> None:
        self.assertEqual(
            ci_test_runner.wasm_stages("wasm-pack-x"),
            (
                ci_test_runner.Stage(
                    "Run wasm-bindgen tests under Node",
                    ("wasm-pack-x", "test", "--node", "crates/velesdb-wasm"),
                ),
            ),
        )


if __name__ == "__main__":
    unittest.main()
