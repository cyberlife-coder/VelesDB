"""Phase C orchestrator — run from the analysis/ dir: python run_phase_c.py"""
import json
import sys

from load import ANSWERABLE, build_paired, load_all, sanity_check
from stats import (attribute_flips, cluster_bootstrap_delta, mcnemar_by_category,
                    per_conversation_table, wilson_table)
from charts import bar_with_ci, flip_flow, per_conv_delta_strip, scaffold_scatter

OUT_DIR = "../out"
RESULT_DIR = "out"
CATEGORIES = ["temporal", "multi-hop", "single-hop", "open-domain", "adversarial"]
CONFIGS = ["base", "dated", "scaffold"]


def main():
    print("Loading + joining JSONLs (graph_on=True)...", file=sys.stderr)
    cfgs = load_all(OUT_DIR, graph_on=True)
    for name, df in cfgs.items():
        print(f"  {name}: {len(df)} rows", file=sys.stderr)
    paired = build_paired(cfgs)
    paired.to_csv(f"{RESULT_DIR}/paired_table.csv.gz", index=False, compression="gzip")

    print("Sanity check vs published/memory table...", file=sys.stderr)
    sanity = sanity_check(paired)
    print(sanity, file=sys.stderr)

    result = {"sanity_check": sanity.to_dict(orient="index")}

    print("McNemar exact: baseline -> dated ...", file=sys.stderr)
    mc_bd = mcnemar_by_category(paired, "correct_base", "correct_dated", CATEGORIES)
    print("McNemar exact: dated -> scaffold ...", file=sys.stderr)
    mc_ds = mcnemar_by_category(paired, "correct_dated", "correct_scaffold", CATEGORIES)
    result["mcnemar_baseline_to_dated"] = mc_bd
    result["mcnemar_dated_to_scaffold"] = mc_ds

    print("Wilson CIs ...", file=sys.stderr)
    wilson_rows = wilson_table(paired, CONFIGS, CATEGORIES)
    ans = paired[paired.category.isin(ANSWERABLE)]
    for cfg in CONFIGS:
        col = f"correct_{cfg}"
        sub = ans[col].dropna()
        k, n = int(sub.sum()), len(sub)
        from stats import wilson_ci
        p, lo, hi = wilson_ci(k, n)
        wilson_rows.append({"config": cfg, "category": "answerable", "n": n, "k": k,
                             "acc": round(p * 100, 1), "ci_lo": round(lo * 100, 1), "ci_hi": round(hi * 100, 1)})
    result["wilson"] = wilson_rows

    print("Cluster bootstrap (10000 resamples over 10 conversations) ...", file=sys.stderr)
    boot_bd = cluster_bootstrap_delta(paired, "correct_base", "correct_dated", CATEGORIES)
    boot_ds = cluster_bootstrap_delta(paired, "correct_dated", "correct_scaffold", CATEGORIES)
    # answerable-level bootstrap needs a synthetic "answerable" category column
    paired["category_ans"] = paired["category"].where(paired["category"].isin(ANSWERABLE), other=None)
    ans_paired = paired.copy()
    ans_paired["category"] = "answerable"
    ans_paired = ans_paired[ans_paired["category_ans"].notna()]
    boot_bd["answerable"] = cluster_bootstrap_delta(ans_paired, "correct_base", "correct_dated", ["answerable"])["answerable"]
    boot_ds["answerable"] = cluster_bootstrap_delta(ans_paired, "correct_dated", "correct_scaffold", ["answerable"])["answerable"]
    result["bootstrap_baseline_to_dated"] = boot_bd
    result["bootstrap_dated_to_scaffold"] = boot_ds

    print("Attribution: temporal baseline->dated, single-hop dated->scaffold ...", file=sys.stderr)
    attr_temporal = attribute_flips(paired, "correct_base", "correct_dated", "temporal",
                                     pred_from="predicted_base", pred_to="predicted_dated",
                                     trigger_col="is_temporal_trigger_dated")
    attr_singlehop = attribute_flips(paired, "correct_dated", "correct_scaffold", "single-hop",
                                      pred_from="predicted_dated", pred_to="predicted_scaffold",
                                      trigger_col="is_temporal_trigger_scaffold")
    result["attribution_temporal_baseline_to_dated"] = attr_temporal
    result["attribution_singlehop_dated_to_scaffold"] = attr_singlehop

    # cross-tab by prompt_kind for the single-hop scaffold flips
    sub = paired[paired.category == "single-hop"].dropna(subset=["correct_dated", "correct_scaffold"])
    lost = sub[sub.correct_dated.astype(bool) & (~sub.correct_scaffold.astype(bool))]
    result["singlehop_lost_by_prompt_kind_scaffold"] = lost["prompt_kind_scaffold"].value_counts(dropna=False).to_dict()

    print("Per-conversation breakdown + outliers ...", file=sys.stderr)
    per_conv = per_conversation_table(paired, CONFIGS, CATEGORIES)
    with open(f"{RESULT_DIR}/per_conversation.json", "w") as f:
        json.dump(per_conv, f, indent=2)
    result["per_conversation_outliers"] = [r for r in per_conv if r["outlier"]]

    with open(f"{RESULT_DIR}/phase_c_summary.json", "w") as f:
        json.dump(result, f, indent=2, default=str)

    print("Charts ...", file=sys.stderr)
    bar_with_ci(wilson_rows, CATEGORIES + ["answerable"], CONFIGS, f"{RESULT_DIR}/bar_ci.png")
    flip_flow({"baseline→dated (temporal)": attr_temporal, "dated→scaffold (single-hop)": attr_singlehop},
              f"{RESULT_DIR}/flip_flow.png")
    per_conv_delta_strip(per_conv, CATEGORIES, CONFIGS, f"{RESULT_DIR}/per_conv_delta.png")
    scaffold_scatter(paired, f"{RESULT_DIR}/scaffold_scatter.png")

    print("Done. Summary: out/phase_c_summary.json, charts in out/*.png", file=sys.stderr)


if __name__ == "__main__":
    main()
