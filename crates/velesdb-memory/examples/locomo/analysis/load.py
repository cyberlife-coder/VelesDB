"""Phase C — load & join the 3 Phase B dump JSONLs into a paired wide table."""
import json

import pandas as pd

FIELDS = [
    "conversation_id", "question_idx", "category", "gold", "predicted",
    "correct", "f1", "evidence_hit", "date_on", "scaffold_on", "prompt_kind",
    "is_temporal_trigger", "prompt_tokens", "completion_tokens",
    "context_tokens", "n_distinct_dates", "date_span_days",
]

ANSWERABLE = ("temporal", "multi-hop", "single-hop", "open-domain")


def load_config(path, graph_on=True):
    """Stream a dump JSONL, keep scalar fields only (drop raw/reranked_facts arrays)."""
    rows = []
    with open(path) as f:
        for line in f:
            d = json.loads(line)
            if d.get("graph_on") != graph_on:
                continue
            rows.append({k: d.get(k) for k in FIELDS})
    return pd.DataFrame(rows)


def load_all(out_dir, graph_on=True):
    cfgs = {}
    for name in ("baseline", "dated", "scaffold"):
        cfgs[name] = load_config(f"{out_dir}/{name}.jsonl", graph_on=graph_on)
    return cfgs


def build_paired(cfgs):
    """Outer-join baseline/dated/scaffold on (conversation_id, question_idx)."""
    key = ["conversation_id", "question_idx", "category"]
    b = cfgs["baseline"].add_suffix("_base")
    d = cfgs["dated"].add_suffix("_dated")
    s = cfgs["scaffold"].add_suffix("_scaffold")
    for df, suf in ((b, "_base"), (d, "_dated"), (s, "_scaffold")):
        df.rename(columns={f"{k}{suf}": k for k in key}, inplace=True)
    paired = b.merge(d, on=key, how="outer", validate="one_to_one")
    paired = paired.merge(s, on=key, how="outer", validate="one_to_one")
    return paired


def sanity_check(paired):
    """Recompute aggregate accuracy per (config, category) — compare to the published/memory table."""
    rows = []
    for cfg in ("base", "dated", "scaffold"):
        col = f"correct_{cfg}"
        acc_by_cat = paired.groupby("category")[col].mean().mul(100).round(0)
        answerable = paired[paired.category.isin(ANSWERABLE)][col].mean() * 100
        rows.append({"config": cfg, **acc_by_cat.to_dict(), "answerable": round(answerable)})
    return pd.DataFrame(rows).set_index("config")
