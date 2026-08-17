#!/usr/bin/env python3
"""CLI entry point for the Linux N-API functional gate."""

from ci_test_runner import run_node_binding


if __name__ == "__main__":
    raise SystemExit(run_node_binding())
