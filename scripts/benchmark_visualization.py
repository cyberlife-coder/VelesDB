#!/usr/bin/env python3
"""
VelesDB Recall vs Latency Visualization
Generates publication-quality charts for benchmark results.
"""

import matplotlib.pyplot as plt
import numpy as np
from dataclasses import dataclass
from typing import List, Optional

@dataclass
class BenchmarkResult:
    mode: str
    ef_search: Optional[int]  # None: Perfect is an exhaustive scan, with no ef
    recall: float  # percentage
    latency_p50_ms: float

# Benchmark data from VelesDB Core - January 11, 2026 (v1.1.0)
# Native HNSW + SIMD intrinsics + Lock-Free Cache + Trigram Index
RESULTS_10K_128D = [
    BenchmarkResult("Fast", 64, 92.2, 0.036),
    BenchmarkResult("Balanced", 128, 98.8, 0.057),
    # 512, not 256: Accurate's effort since 9c222258 (2026-01-09) (#2250).
    BenchmarkResult("Accurate", 512, 100.0, 0.130),
    BenchmarkResult("Perfect", None, 100.0, 0.200),
]

@dataclass
class NativeVsHnswRsResult:
    operation: str
    native_ms: float
    hnsw_rs_ms: float

# Native HNSW vs hnsw_rs comparison - January 8, 2026
# 5,000 vectors, 128D, Euclidean distance
NATIVE_VS_HNSW_RS = [
    NativeVsHnswRsResult("Search (100q)", 26.9, 32.4),
    NativeVsHnswRsResult("Parallel Insert", 1470.0, 1570.0),  # in ms for consistency
]

def create_recall_latency_chart(results: List[BenchmarkResult], title: str, filename: str):
    """Create a recall vs latency chart with annotations."""
    
    fig, ax = plt.subplots(figsize=(12, 8))
    
    recalls = [r.recall for r in results]
    latencies = [r.latency_p50_ms for r in results]
    modes = [r.mode for r in results]
    ef_values = [r.ef_search for r in results]
    
    # Main curve
    ax.plot(latencies, recalls, 'o-', linewidth=2.5, markersize=12, 
            color='#2563eb', markerfacecolor='white', markeredgewidth=2.5)
    
    # Annotations for each point
    for i, (lat, rec, mode, ef) in enumerate(zip(latencies, recalls, modes, ef_values)):
        offset = (10, 10) if i % 2 == 0 else (10, -15)
        ef_label = 'exhaustive' if ef is None else f'ef={ef}'
        ax.annotate(f'{mode}\n{ef_label}\n{rec:.1f}%', 
                   (lat, rec), 
                   textcoords="offset points",
                   xytext=offset,
                   fontsize=9,
                   ha='left',
                   bbox=dict(boxstyle='round,pad=0.3', facecolor='white', edgecolor='gray', alpha=0.8))
    
    # Target zones
    ax.axhline(y=95, color='green', linestyle='--', alpha=0.5, label='Production target (95%)')
    ax.axhline(y=99, color='orange', linestyle='--', alpha=0.5, label='High-quality target (99%)')
    
    # Styling
    ax.set_xlabel('Latency P50 (ms)', fontsize=14, fontweight='bold')
    ax.set_ylabel('Recall@10 (%)', fontsize=14, fontweight='bold')
    ax.set_title(title, fontsize=16, fontweight='bold', pad=20)
    
    ax.set_ylim(88, 101)
    ax.set_xlim(0, max(latencies) * 1.5)
    
    ax.grid(True, alpha=0.3)
    ax.legend(loc='lower right', fontsize=10)
    
    # Add VelesDB branding
    fig.text(0.99, 0.01, 'VelesDB Core v1.1.0 - January 11, 2026', fontsize=8, 
             ha='right', va='bottom', alpha=0.5, style='italic')
    
    plt.tight_layout()
    plt.savefig(filename, dpi=150, bbox_inches='tight', facecolor='white')
    plt.close()
    print(f"✅ Chart saved: {filename}")

def create_native_hnsw_comparison(results: List[NativeVsHnswRsResult], filename: str):
    """Create a bar chart comparing Native HNSW vs hnsw_rs."""
    
    fig, ax = plt.subplots(figsize=(10, 7))
    
    operations = [r.operation for r in results]
    native_times = [r.native_ms for r in results]
    hnsw_rs_times = [r.hnsw_rs_ms for r in results]
    
    x = np.arange(len(operations))
    width = 0.35
    
    bars1 = ax.bar(x - width/2, native_times, width, label='Native HNSW', color='#2563eb')
    bars2 = ax.bar(x + width/2, hnsw_rs_times, width, label='hnsw_rs', color='#dc2626')
    
    # Add percentage improvement labels
    for i, (native, hnsw) in enumerate(zip(native_times, hnsw_rs_times)):
        improvement = ((hnsw - native) / hnsw) * 100
        ax.annotate(f'{improvement:.0f}% faster', 
                   xy=(x[i] - width/2, native + max(native_times) * 0.02),
                   ha='center', fontsize=10, fontweight='bold', color='#16a34a')
    
    ax.set_ylabel('Time (ms)', fontsize=14, fontweight='bold')
    ax.set_title('VelesDB Native HNSW vs hnsw_rs\n(5K vectors, 128D, Euclidean)', 
                fontsize=14, fontweight='bold')
    ax.set_xticks(x)
    ax.set_xticklabels(operations, fontsize=12)
    ax.legend(fontsize=11)
    ax.grid(True, alpha=0.3, axis='y')
    
    # Add value labels on bars
    for bar in bars1:
        height = bar.get_height()
        ax.annotate(f'{height:.1f}ms' if height < 100 else f'{height/1000:.2f}s',
                   xy=(bar.get_x() + bar.get_width()/2, height),
                   xytext=(0, 3), textcoords="offset points",
                   ha='center', va='bottom', fontsize=9)
    
    for bar in bars2:
        height = bar.get_height()
        ax.annotate(f'{height:.1f}ms' if height < 100 else f'{height/1000:.2f}s',
                   xy=(bar.get_x() + bar.get_width()/2, height),
                   xytext=(0, 3), textcoords="offset points",
                   ha='center', va='bottom', fontsize=9)
    
    fig.text(0.99, 0.01, 'VelesDB Core v1.1.0 - January 11, 2026', fontsize=8, 
             ha='right', va='bottom', alpha=0.5, style='italic')
    
    plt.tight_layout()
    plt.savefig(filename, dpi=150, bbox_inches='tight', facecolor='white')
    plt.close()
    print(f"✅ Native HNSW comparison chart saved: {filename}")

def create_ef_scaling_chart(results: List[BenchmarkResult], filename: str):
    """Show how ef_search affects both recall and latency."""
    
    fig, ax1 = plt.subplots(figsize=(12, 7))
    
    # Perfect sets no ef_search -- it is an exhaustive scan -- so it has no
    # place on an ef axis. Plot only the modes that set one.
    swept = sorted((r for r in results if r.ef_search is not None),
                   key=lambda r: r.ef_search)
    ef_values = [r.ef_search for r in swept]
    recalls = [r.recall for r in swept]
    latencies = [r.latency_p50_ms for r in swept]
    
    # Recall curve (left y-axis)
    color1 = '#2563eb'
    ax1.set_xlabel('ef_search', fontsize=14, fontweight='bold')
    ax1.set_ylabel('Recall@10 (%)', fontsize=14, fontweight='bold', color=color1)
    line1 = ax1.plot(ef_values, recalls, 'o-', linewidth=2.5, markersize=10, 
                     color=color1, label='Recall@10')
    ax1.tick_params(axis='y', labelcolor=color1)
    ax1.set_ylim(80, 101)
    ax1.set_xscale('log', base=2)
    
    # Latency curve (right y-axis)
    ax2 = ax1.twinx()
    color2 = '#dc2626'
    ax2.set_ylabel('Latency P50 (ms)', fontsize=14, fontweight='bold', color=color2)
    line2 = ax2.plot(ef_values, latencies, 's--', linewidth=2.5, markersize=10, 
                     color=color2, label='Latency P50')
    ax2.tick_params(axis='y', labelcolor=color2)
    
    # Combined legend
    lines = line1 + line2
    labels = [l.get_label() for l in lines]
    ax1.legend(lines, labels, loc='center right', fontsize=11)
    
    ax1.set_title('VelesDB Core - ef_search Scaling Behavior\n(10K vectors / 128D, v1.1.0)', 
                  fontsize=14, fontweight='bold', pad=15)
    ax1.grid(True, alpha=0.3)
    
    # Computed from the points plotted, so the caption cannot outlive its data;
    # with fewer than two of them, or no latency to divide by, there is no
    # ratio to state.
    if len(swept) >= 2 and swept[0].latency_p50_ms > 0:
        low, high = swept[0], swept[-1]
        fig.text(0.5, 0.02,
                 f'💡 {high.ef_search / low.ef_search:.1f}x ef_search '
                 f'({low.ef_search}→{high.ef_search}) costs '
                 f'{high.latency_p50_ms / low.latency_p50_ms:.1f}x P50 latency '
                 f'for {high.recall - low.recall:+.1f} recall points',
                 fontsize=11, ha='center', style='italic',
                 bbox=dict(boxstyle='round', facecolor='#f0f9ff', edgecolor='#2563eb'))
    
    plt.tight_layout()
    plt.savefig(filename, dpi=150, bbox_inches='tight', facecolor='white')
    plt.close()
    print(f"✅ Scaling chart saved: {filename}")

if __name__ == "__main__":
    import os
    
    output_dir = os.path.dirname(os.path.abspath(__file__))
    charts_dir = os.path.join(output_dir, "..", "docs", "benchmarks")
    os.makedirs(charts_dir, exist_ok=True)
    
    print("🎨 Generating VelesDB benchmark visualizations...\n")
    
    # Generate charts
    create_recall_latency_chart(
        RESULTS_10K_128D, 
        "VelesDB Core - Recall vs Latency (10K/128D)",
        os.path.join(charts_dir, "recall_latency_10k_128d.png")
    )
    
    create_ef_scaling_chart(
        RESULTS_10K_128D,
        os.path.join(charts_dir, "ef_scaling_10k_128d.png")
    )
    
    create_native_hnsw_comparison(
        NATIVE_VS_HNSW_RS,
        os.path.join(charts_dir, "native_hnsw_comparison.png")
    )
    
    print("\n✅ All charts generated successfully!")
    print(f"📁 Output directory: {charts_dir}")
