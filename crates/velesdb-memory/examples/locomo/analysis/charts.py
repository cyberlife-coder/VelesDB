"""Phase C — matplotlib charts (grouped bars w/ CI, flip-flow, per-conv delta, scaffold scatter)."""
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np


def bar_with_ci(wilson_rows, categories, configs, out_path):
    x = np.arange(len(categories))
    width = 0.25
    fig, ax = plt.subplots(figsize=(9, 5))
    by_cfg = {cfg: {r["category"]: r for r in wilson_rows if r["config"] == cfg} for cfg in configs}
    for i, cfg in enumerate(configs):
        accs = [by_cfg[cfg][c]["acc"] for c in categories]
        los = [by_cfg[cfg][c]["acc"] - by_cfg[cfg][c]["ci_lo"] for c in categories]
        his = [by_cfg[cfg][c]["ci_hi"] - by_cfg[cfg][c]["acc"] for c in categories]
        ax.bar(x + (i - 1) * width, accs, width, yerr=[los, his], capsize=3, label=cfg)
    ax.set_xticks(x)
    ax.set_xticklabels(categories, rotation=20)
    ax.set_ylabel("Accuracy (%)")
    ax.set_title("Accuracy by category and config (Wilson 95% CI)")
    ax.legend()
    fig.tight_layout()
    fig.savefig(out_path, dpi=130)
    plt.close(fig)


def flip_flow(attributions, out_path):
    """attributions: dict of transition_label -> attribute_flips() result."""
    labels = list(attributions.keys())
    won = [attributions[k]["won_wrong_to_correct"] for k in labels]
    lost = [attributions[k]["lost_correct_to_wrong"] for k in labels]
    x = np.arange(len(labels))
    width = 0.35
    fig, ax = plt.subplots(figsize=(7, 5))
    ax.bar(x - width / 2, won, width, label="won (wrong→correct)", color="#2a9d8f")
    ax.bar(x + width / 2, lost, width, label="lost (correct→wrong)", color="#e76f51")
    ax.set_xticks(x)
    ax.set_xticklabels(labels, rotation=15)
    ax.set_ylabel("# questions")
    ax.set_title("Flip flow: won vs lost per transition")
    ax.legend()
    fig.tight_layout()
    fig.savefig(out_path, dpi=130)
    plt.close(fig)


def per_conv_delta_strip(per_conv_rows, categories, configs, out_path):
    fig, axes = plt.subplots(1, len(categories), figsize=(4 * len(categories), 4.5), sharey=True)
    if len(categories) == 1:
        axes = [axes]
    for ax, cat in zip(axes, categories):
        data = []
        for cfg in configs:
            vals = [r["acc"] for r in per_conv_rows if r["category"] == cat and r["config"] == cfg]
            data.append(vals)
        ax.boxplot(data, tick_labels=configs, showmeans=True)
        for i, vals in enumerate(data, start=1):
            jitter = np.random.default_rng(0).normal(0, 0.04, size=len(vals))
            ax.scatter(np.full(len(vals), i) + jitter, vals, alpha=0.6, s=18, color="#264653")
        ax.set_title(cat)
        ax.tick_params(axis="x", rotation=20)
    axes[0].set_ylabel("Per-conversation accuracy (%)")
    fig.suptitle("Per-conversation accuracy spread by config")
    fig.tight_layout()
    fig.savefig(out_path, dpi=130)
    plt.close(fig)


def scaffold_scatter(paired, out_path):
    sub = paired[paired.category == "single-hop"].dropna(
        subset=["correct_dated", "correct_scaffold", "n_distinct_dates_scaffold", "date_span_days_scaffold"]
    ).copy()
    won = sub[(~sub.correct_dated.astype(bool)) & sub.correct_scaffold.astype(bool)]
    lost = sub[sub.correct_dated.astype(bool) & (~sub.correct_scaffold.astype(bool))]
    unchanged = sub[sub.correct_dated.astype(bool) == sub.correct_scaffold.astype(bool)]
    fig, ax = plt.subplots(figsize=(7, 5.5))
    ax.scatter(unchanged.n_distinct_dates_scaffold, unchanged.date_span_days_scaffold,
               alpha=0.25, s=14, color="#adb5bd", label="unchanged")
    ax.scatter(won.n_distinct_dates_scaffold, won.date_span_days_scaffold,
               alpha=0.9, s=40, color="#2a9d8f", label="won (dated→scaffold)", marker="^")
    ax.scatter(lost.n_distinct_dates_scaffold, lost.date_span_days_scaffold,
               alpha=0.9, s=40, color="#e76f51", label="lost (dated→scaffold)", marker="x")
    ax.set_xlabel("n_distinct_dates")
    ax.set_ylabel("date_span_days")
    ax.set_title("Single-hop scaffold flips vs. temporal context load")
    ax.legend()
    fig.tight_layout()
    fig.savefig(out_path, dpi=130)
    plt.close(fig)
