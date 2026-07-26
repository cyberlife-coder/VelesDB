#!/usr/bin/env python3
"""Render the core-parity matrix from one or more workflow result JSON files.

Each input JSON is the structured result of the map→verify workflow(s):
    { "catalog": [ {id, group, name, core_symbols, ...}, ... ],   # at least one input carries it
      "components": [ {key, label, rows:[{capability_id,status,evidence,notes}], extra_surface, ...}, ... ] }

Usage:
    gen_matrix.py OUT.md RESULT1.json [RESULT2.json ...]

Status glyphs: full=✅  partial=⚠️  absent=❌  na=·
The skill (core-parity-audit) explains how the result JSONs are produced.
"""
import json
import sys
from collections import Counter

GLYPH = {"full": "✅", "partial": "⚠️", "absent": "❌", "na": "·"}

# canonical column order + display labels (extend if new components appear)
ORDER = ["server", "cli", "python", "wasm", "mobile", "migrate", "tauri",
         "ts-sdk", "langchain", "llamaindex", "haystack", "common", "docs"]
LABELS = {"server": "Server", "cli": "CLI", "python": "Python", "wasm": "WASM",
          "mobile": "Mobile", "migrate": "Migrate", "tauri": "Tauri", "ts-sdk": "TS SDK",
          "langchain": "LangChain", "llamaindex": "LlamaIndex", "haystack": "Haystack",
          "common": "int/common", "docs": "Docs"}

# Core-internal / ops domains: never expected to cross the binding boundary, so
# they are excluded from the "user-facing coverage" denominator.
INTERNAL_GROUPS = {"Observability", "Config surface", "Persistence",
                   "Validation & guardrails", "Database lifecycle",
                   "Query planning / introspection", "Indexes"}


def load_inputs(paths):
    catalog, by_key = [], {}
    seen_caps = set()
    for p in paths:
        doc = json.load(open(p))
        doc = doc.get("result", doc)
        if isinstance(doc, str):
            doc = json.loads(doc)
        for cap in doc.get("catalog", []) or []:
            if cap.get("id") and cap["id"] not in seen_caps:
                seen_caps.add(cap["id"])
                catalog.append(cap)
        for comp in doc.get("components", []) or []:
            by_key[comp["key"]] = comp  # later file wins on dup key
    return catalog, by_key


def status_of(comp, cid):
    for row in comp.get("rows", []):
        if row["capability_id"] == cid:
            return row["status"]
    return "na"


def main():
    if len(sys.argv) < 3:
        sys.exit("usage: gen_matrix.py OUT.md RESULT1.json [RESULT2.json ...]")
    out_path, inputs = sys.argv[1], sys.argv[2:]
    catalog, by_key = load_inputs(inputs)
    cat = {c["id"]: c for c in catalog}
    present = [k for k in ORDER if k in by_key] + [k for k in by_key if k not in ORDER]

    groups = []
    for c in catalog:
        if c["group"] not in groups:
            groups.append(c["group"])

    out = []
    out.append("# VelesDB Core → Ecosystem Public-API Parity Matrix\n")
    out.append(f"**Source of truth:** `velesdb-core` — **{len(catalog)} capabilities** across {len(groups)} domains, "
               f"× {len(present)} components. Every gap adversarially re-verified.\n")
    out.append("Legend: ✅ full · ⚠️ partial / reduced · ❌ absent (plausible gap) · · N/A by design\n")

    hdr = "| Capability | " + " | ".join(LABELS.get(k, k) for k in present) + " |"
    sep = "|" + "---|" * (len(present) + 1)
    for g in groups:
        out.append(f"\n### {g}\n")
        out.append(hdr)
        out.append(sep)
        for c in catalog:
            if c["group"] != g:
                continue
            cells = [GLYPH.get(status_of(by_key[k], c["id"]), "?") for k in present]
            out.append(f"| {c['name']} | " + " | ".join(cells) + " |")

    out.append("\n## Coverage scorecard (per component)\n")
    out.append("| Component | full | partial | absent | n/a | user-facing coverage* |")
    out.append("|---|---|---|---|---|---|")
    for k in present:
        comp = by_key[k]
        st = Counter(r["status"] for r in comp.get("rows", []))
        uf_full = uf_part = uf_abs = 0
        for r in comp.get("rows", []):
            grp = cat.get(r["capability_id"], {}).get("group", "")
            if grp and grp not in INTERNAL_GROUPS:
                uf_full += r["status"] == "full"
                uf_part += r["status"] == "partial"
                uf_abs += r["status"] == "absent"
        denom = uf_full + uf_part + uf_abs
        cov = f"{round(100 * (uf_full + 0.5 * uf_part) / denom)}%" if denom else "n/a"
        out.append(f"| {LABELS.get(k, k)} | {st.get('full', 0)} | {st.get('partial', 0)} | "
                   f"{st.get('absent', 0)} | {st.get('na', 0)} | {cov} |")
    out.append("\n*User-facing coverage = (full + ½·partial) / (full+partial+absent) over user-facing domains "
               "(excludes core-internal observability/config/durability/registry plumbing).\n")

    open(out_path, "w").write("\n".join(out))

    # gap buckets to stderr (so chat synthesis can read them)
    uf, internal = [], []
    for k in present:
        for r in by_key[k].get("rows", []):
            if r["status"] != "absent":
                continue
            grp = cat.get(r["capability_id"], {}).get("group", "")
            (internal if grp in INTERNAL_GROUPS else uf).append((k, r["capability_id"]))
    print(f"wrote {out_path}: {len(catalog)} caps × {len(present)} components", file=sys.stderr)
    print(f"absent — user-facing: {len(uf)}  core-internal/ops: {len(internal)}", file=sys.stderr)
    print("\nuser-facing absent by capability (missing in →):", file=sys.stderr)
    for cid, _ in Counter(c for _, c in uf).most_common():
        comps = [k for k, cc in uf if cc == cid]
        print(f"  {cid:38} {', '.join(comps)}", file=sys.stderr)


if __name__ == "__main__":
    main()
