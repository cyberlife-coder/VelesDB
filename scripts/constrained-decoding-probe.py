#!/usr/bin/env python3
"""Settle one question with a real model: does `format` keep what it does not name?

The extraction campaign (#1943) measured constrained decoding with a schema that
constrains `relations` and `attributes` only — `facts` appears in neither
`properties` nor `required`. The crate now sends a **superset** of that schema
which also constrains `facts` (#1944), on the reasoning that a declared property
can only help a grammar keep it.

That reasoning is an assumption about llama.cpp's JSON-schema-to-grammar step,
and it decides whether the campaign's schema would have silently dropped every
fact — the primary output of extraction. It cannot be settled by reading the
crate, and it does not depend on the model: it is a property of the grammar
compiler, so the smallest model that answers reliably is the right instrument.
A GPU tier is irrelevant here, which is what makes this runnable on CI at all.

The prompt is read out of `extract.rs` through the campaign's own harness, so
this measures the prompt the product actually sends.

Exit code is 0 when the run produced a verdict, 1 when it could not — never
"1 because the answer was inconvenient". The verdict is printed, not encoded in
the status, because both outcomes are informative.
"""

from __future__ import annotations

import argparse
import copy
import importlib.util
import json
import sys
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BENCH = ROOT / "scripts" / "bench-memory-extraction.py"

# Two passages, one per language the campaign scores, each stating facts AND a
# relation AND an attribute — so a dropped section is visible rather than
# ambiguous.
PASSAGES = [
    "Zephyrin Marchandeau travaille chez Wiscale depuis 2019. Il a un fils, "
    "Kaltar, qui a 15 ans et vit a Nantes.",
    "Priya Raghunathan joined Orbital Freight in 2021 as a staff engineer. "
    "Her sister Meena is 28 and lives in Bristol.",
]


def load_bench():
    """Import the campaign harness by path — it is a script, not a module."""
    spec = importlib.util.spec_from_file_location("bench_extraction", BENCH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {BENCH}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def superset_of(subset: dict) -> dict:
    """The campaign's schema plus the `facts` section the crate now declares.

    Built FROM the subset rather than typed out, so the only difference between
    the two arms is the one under test.
    """
    schema = copy.deepcopy(subset)
    schema["properties"]["facts"] = {
        "type": "array",
        "items": {
            "type": "object",
            "properties": {
                "fact": {"type": "string"},
                "entities": {"type": "array", "items": {"type": "string"}},
            },
            "required": ["fact", "entities"],
        },
    }
    schema["required"] = ["facts", *subset["required"]]
    return schema


def generate(url: str, model: str, prompt: str, schema: dict | None,
             num_ctx: int, timeout: float) -> str:
    body = {
        "model": model,
        "prompt": prompt,
        "stream": False,
        "think": False,
        "keep_alive": -1,
        "options": {"temperature": 0, "num_predict": 1024, "num_ctx": num_ctx},
    }
    if schema is not None:
        body["format"] = schema
    request = urllib.request.Request(
        f"{url.rstrip('/')}/api/generate",
        data=json.dumps(body).encode("utf-8"),
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return json.loads(response.read().decode("utf-8")).get("response", "")


def summarise(reply: str) -> dict:
    """What the reply carries, or why it carries nothing."""
    try:
        payload = json.loads(reply)
    except (json.JSONDecodeError, ValueError):
        return {"parsed": False, "facts": None, "relations": None,
                "attributes": None, "keys": None}
    if not isinstance(payload, dict):
        return {"parsed": True, "facts": None, "relations": None,
                "attributes": None, "keys": f"<{type(payload).__name__}>"}

    def count(key: str) -> int | None:
        value = payload.get(key)
        return len(value) if isinstance(value, list) else None

    return {
        "parsed": True,
        "facts": count("facts"),
        "relations": count("relations"),
        "attributes": count("attributes"),
        "keys": ",".join(sorted(payload)),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="http://localhost:11434")
    parser.add_argument("--model", required=True)
    parser.add_argument("--num-ctx", type=int, default=4096, dest="num_ctx")
    parser.add_argument("--timeout", type=float, default=900.0)
    parser.add_argument("--out", help="write the raw run as JSON here")
    args = parser.parse_args()

    bench = load_bench()
    template = bench.read_graph_prompt_template()
    subset = bench.EXTRACTION_SCHEMA
    arms = {
        "unconstrained": None,
        "campaign-subset": subset,
        "shipped-superset": superset_of(subset),
    }

    runs = []
    for index, passage in enumerate(PASSAGES):
        prompt = bench.build_graph_prompt(passage, template)
        for arm, schema in arms.items():
            try:
                reply = generate(args.url, args.model, prompt, schema,
                                 args.num_ctx, args.timeout)
            except (urllib.error.URLError, TimeoutError, OSError) as err:
                print(f"FAILED to reach ollama for {arm} on passage {index}: {err}",
                      file=sys.stderr)
                return 1
            record = {"passage": index, "arm": arm, **summarise(reply),
                      "reply": reply}
            runs.append(record)
            print(f"passage {index} | {arm:17} | parsed={record['parsed']} "
                  f"facts={record['facts']} relations={record['relations']} "
                  f"attributes={record['attributes']} keys={record['keys']}")

    print()
    print("=== verdict ===")
    subset_runs = [run for run in runs if run["arm"] == "campaign-subset"]
    superset_runs = [run for run in runs if run["arm"] == "shipped-superset"]
    subset_kept_facts = [run["facts"] for run in subset_runs]
    superset_kept_facts = [run["facts"] for run in superset_runs]

    if all(count in (None, 0) for count in subset_kept_facts):
        print("The campaign's schema DROPS facts: a property it does not declare "
              "does not survive the grammar.")
        print("=> constraining `facts` is not merely safe, it is REQUIRED, and "
              "shipping the campaign's schema verbatim would have silently "
              "emptied every extraction.")
    elif all(isinstance(count, int) and count > 0 for count in subset_kept_facts):
        print("The campaign's schema KEEPS facts: undeclared properties survive "
              "the grammar on this Ollama build.")
        print("=> the superset is a belt-and-braces choice, not a correction. "
              "Its cost is that the published tier table was measured without "
              "it; its benefit is not depending on that behaviour.")
    else:
        print(f"MIXED: facts under the campaign schema = {subset_kept_facts}. "
              "Not a stable property on this build — which is itself a reason "
              "to declare the section rather than rely on it.")

    print(f"facts under the shipped superset = {superset_kept_facts}")
    if any(not run["parsed"] for run in superset_runs):
        print("WARNING: the shipped superset produced an unparsable reply — "
              "that is a defect in the schema this crate sends.")

    if args.out:
        Path(args.out).write_text(json.dumps(runs, indent=2, ensure_ascii=False),
                                  encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
