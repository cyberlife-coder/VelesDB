"""Phase C — paired McNemar tests, Wilson CIs, cluster bootstrap, flip attribution."""
import numpy as np
from scipy.stats import binomtest


def mcnemar_exact(correct_a, correct_b):
    """Exact McNemar (binomial on discordant pairs). Returns b/c discordant counts + p-value.
    b = a-correct/b-wrong, c = a-wrong/b-correct (standard McNemar notation)."""
    a = correct_a.astype(bool).to_numpy()
    b_arr = correct_b.astype(bool).to_numpy()
    b = int(np.sum(a & ~b_arr))   # correct -> wrong
    c = int(np.sum(~a & b_arr))   # wrong -> correct
    n = b + c
    if n == 0:
        return {"b_correct_to_wrong": b, "c_wrong_to_correct": c, "n_discordant": 0, "p_value": 1.0}
    p = binomtest(min(b, c), n, 0.5, alternative="two-sided").pvalue
    return {"b_correct_to_wrong": b, "c_wrong_to_correct": c, "n_discordant": n, "p_value": p}


def mcnemar_by_category(paired, col_a, col_b, categories):
    out = {}
    for cat in categories:
        sub = paired[paired.category == cat].dropna(subset=[col_a, col_b])
        out[cat] = mcnemar_exact(sub[col_a], sub[col_b])
    return out


def wilson_ci(k, n, z=1.96):
    """Wilson score interval for a binomial proportion."""
    if n == 0:
        return (float("nan"), float("nan"), float("nan"))
    p = k / n
    denom = 1 + z ** 2 / n
    center = (p + z ** 2 / (2 * n)) / denom
    half = (z * np.sqrt(p * (1 - p) / n + z ** 2 / (4 * n ** 2))) / denom
    return (p, max(0.0, center - half), min(1.0, center + half))


def wilson_table(paired, configs, categories):
    rows = []
    for cfg in configs:
        col = f"correct_{cfg}"
        for cat in categories:
            sub = paired[paired.category == cat][col].dropna()
            k, n = int(sub.sum()), len(sub)
            p, lo, hi = wilson_ci(k, n)
            rows.append({"config": cfg, "category": cat, "n": n, "k": k,
                         "acc": round(p * 100, 1), "ci_lo": round(lo * 100, 1), "ci_hi": round(hi * 100, 1)})
    return rows


def cluster_bootstrap_delta(paired, col_from, col_to, categories, n_boot=10000, seed=0):
    """Resample conversation IDs with replacement; delta = mean(col_to) - mean(col_from) per resample."""
    rng = np.random.default_rng(seed)
    conv_ids = paired["conversation_id"].unique()
    out = {}
    for cat in categories:
        sub = paired[paired.category == cat].dropna(subset=[col_from, col_to])
        by_conv = {cid: g for cid, g in sub.groupby("conversation_id")}
        present = [c for c in conv_ids if c in by_conv]
        deltas = np.empty(n_boot)
        for i in range(n_boot):
            sample = rng.choice(present, size=len(present), replace=True)
            frames = [by_conv[c] for c in sample]
            cat_df = np.concatenate([f[col_from].to_numpy() for f in frames])
            cat_df2 = np.concatenate([f[col_to].to_numpy() for f in frames])
            deltas[i] = cat_df2.mean() - cat_df.mean()
        lo, hi = np.percentile(deltas, [2.5, 97.5])
        point = sub[col_to].mean() - sub[col_from].mean()
        out[cat] = {"delta_pp": round(point * 100, 1), "ci_lo_pp": round(lo * 100, 1), "ci_hi_pp": round(hi * 100, 1)}
    return out


def attribute_flips(paired, col_from, col_to, category, pred_from=None, pred_to=None, trigger_col="is_temporal_trigger_dated"):
    sub = paired[paired.category == category].dropna(subset=[col_from, col_to]).copy()
    from_c, to_c = sub[col_from].astype(bool), sub[col_to].astype(bool)
    won = sub[~from_c & to_c]
    lost = sub[from_c & ~to_c]
    unchanged_correct = sub[from_c & to_c]
    unchanged_wrong = sub[~from_c & ~to_c]
    result = {
        "n_total": len(sub),
        "won_wrong_to_correct": len(won),
        "lost_correct_to_wrong": len(lost),
        "unchanged_correct": len(unchanged_correct),
        "unchanged_wrong": len(unchanged_wrong),
    }
    if trigger_col in sub.columns:
        result["lost_by_trigger"] = lost[trigger_col].value_counts(dropna=False).to_dict()
        result["won_by_trigger"] = won[trigger_col].value_counts(dropna=False).to_dict()
    if pred_from and pred_to and pred_from in sub.columns and pred_to in sub.columns:
        pred_changed_lost = (lost[pred_from] != lost[pred_to]).sum()
        pred_changed_won = (won[pred_from] != won[pred_to]).sum()
        result["lost_predicted_changed"] = int(pred_changed_lost)
        result["lost_predicted_same"] = int(len(lost) - pred_changed_lost)
        result["won_predicted_changed"] = int(pred_changed_won)
        result["won_predicted_same"] = int(len(won) - pred_changed_won)
    return result


def per_conversation_table(paired, configs, categories):
    rows = []
    for cfg in configs:
        col = f"correct_{cfg}"
        g = paired.groupby(["category", "conversation_id"])[col].mean().mul(100)
        for cat in categories:
            vals = g.loc[cat] if cat in g.index.get_level_values(0) else None
            if vals is None or len(vals) == 0:
                continue
            q1, q3 = np.percentile(vals, [25, 75])
            iqr = q3 - q1
            for conv_id, v in vals.items():
                outlier = bool(v < q1 - 1.5 * iqr or v > q3 + 1.5 * iqr)
                rows.append({"config": cfg, "category": cat, "conversation_id": conv_id,
                             "acc": round(v, 1), "outlier": outlier})
    return rows
