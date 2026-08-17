#!/usr/bin/env python3
"""CLI entry point for the wasm32-only wasm-bindgen functional gate."""

from ci_test_runner import run_wasm_bindgen


if __name__ == "__main__":
    raise SystemExit(run_wasm_bindgen())
