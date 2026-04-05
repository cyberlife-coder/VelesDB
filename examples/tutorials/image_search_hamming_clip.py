"""
VelesDB Tutorial: Two-Pass Image Search with Hamming + CLIP

Find duplicate and similar photos in 0.3ms using a combined pipeline:
  Pass 1 - Perceptual hashing (dHash) + Hamming distance for fast shortlisting
  Pass 2 - CLIP embeddings + Euclidean distance for semantic re-ranking

Requirements:
    pip install velesdb Pillow imagehash numpy open-clip-torch torch

Companion article:
    Dev.to:    https://dev.to/wiscale
    Hashnode:  https://hashnode.com/@cyberlifecoder
    GitHub:    https://github.com/cyberlife-coder/VelesDB
    Docs:      https://velesdb.com/en/

VelesDB is source-available under the Elastic License 2.0.
GitHub stars welcome: https://github.com/cyberlife-coder/VelesDB

Author: Julien Lange (WiScale France)
"""

import os
import sys
import time
import shutil
import base64
import urllib.request
from io import BytesIO

import numpy as np
from PIL import Image
import imagehash
import velesdb

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

PHOTO_DIR = os.environ.get("PHOTO_DIR", "./demo_photos")
DB_DIR = os.environ.get("DB_DIR", "./velesdb_image_search")
HASH_SIZE = 16  # dHash dimension: 16x16 = 256-bit vector
CLIP_DIM = 512  # ViT-B-32 output dimension
SHORTLIST_K = 10  # Pass 1 candidates
FINAL_K = 5  # Final results


# ---------------------------------------------------------------------------
# Step 1: Perceptual hashing (dHash) - binary fingerprint per image
# ---------------------------------------------------------------------------

def dhash_vector(img_path: str, hash_size: int = HASH_SIZE) -> list[float]:
    """Compute a dHash perceptual hash and return it as a binary vector.

    dHash (difference hash) works by:
      1. Resizing the image to (hash_size+1) x hash_size grayscale
      2. Comparing adjacent pixel intensities left-to-right
      3. Producing a binary vector: 1 if left > right, else 0

    The result is invariant to resizing, minor cropping, and compression.
    """
    img = Image.open(img_path)
    h = imagehash.dhash(img, hash_size=hash_size)
    return [float(b) for b in h.hash.flatten()]


def index_hashes(db: velesdb.Database, photo_dir: str) -> tuple:
    """Index all images in photo_dir with dHash into a Hamming collection."""
    hash_dim = HASH_SIZE * HASH_SIZE  # 256
    hash_col = db.get_or_create_collection(
        "perceptual_hashes", dimension=hash_dim, metric="hamming"
    )

    files = sorted(
        f for f in os.listdir(photo_dir)
        if f.lower().endswith((".jpg", ".jpeg", ".png", ".webp"))
    )
    if not files:
        print(f"No images found in {photo_dir}")
        sys.exit(1)

    t0 = time.time()
    for i, fname in enumerate(files):
        path = os.path.join(photo_dir, fname)
        vec = dhash_vector(path)
        hash_col.upsert(i + 1, vector=vec, payload={"filename": fname, "path": path})
    elapsed_ms = (time.time() - t0) * 1000

    print(f"  Indexed {len(files)} perceptual hashes in {elapsed_ms:.1f}ms")
    return hash_col, files


# ---------------------------------------------------------------------------
# Step 2: CLIP embeddings - semantic understanding per image
# ---------------------------------------------------------------------------

def load_clip_model():
    """Load CLIP ViT-B-32 model for image embedding."""
    try:
        import open_clip
        import torch
    except ImportError:
        print("open-clip-torch is required for CLIP embeddings.")
        print("Install: pip install open-clip-torch torch")
        sys.exit(1)

    model, _, preprocess = open_clip.create_model_and_transforms(
        "ViT-B-32", pretrained="laion2b_s34b_b79k"
    )
    model.eval()
    return model, preprocess


def clip_embedding(img_path: str, model, preprocess) -> list[float]:
    """Compute a normalized CLIP embedding for an image.

    CLIP (Contrastive Language-Image Pre-training) encodes images and text
    into a shared 512-dimensional space. Two images with similar semantic
    content will have embeddings close together, even if their pixels differ.
    """
    import torch

    img = Image.open(img_path).convert("RGB")
    with torch.no_grad():
        tensor = preprocess(img).unsqueeze(0)
        features = model.encode_image(tensor)
        features /= features.norm(dim=-1, keepdim=True)
        return features.squeeze().numpy().tolist()


def index_clip(db: velesdb.Database, photo_dir: str, files: list, model, preprocess) -> tuple:
    """Index all images with CLIP embeddings into a Euclidean collection."""
    clip_col = db.get_or_create_collection(
        "clip_features", dimension=CLIP_DIM, metric="euclidean"
    )

    t0 = time.time()
    for i, fname in enumerate(files):
        path = os.path.join(photo_dir, fname)
        emb = clip_embedding(path, model, preprocess)
        clip_col.upsert(i + 1, vector=emb, payload={"filename": fname, "path": path})
    elapsed = time.time() - t0

    print(f"  Indexed {len(files)} CLIP embeddings in {elapsed:.1f}s")
    return clip_col


# ---------------------------------------------------------------------------
# Step 3: Two-pass search pipeline
# ---------------------------------------------------------------------------

def two_pass_search(
    query_path: str,
    hash_col,
    clip_col,
    model,
    preprocess,
    shortlist_k: int = SHORTLIST_K,
    final_k: int = FINAL_K,
) -> dict:
    """Execute a two-pass image similarity search.

    Pass 1 (Hamming): ultra-fast shortlisting using perceptual hashes.
        Finds visually similar images by counting differing bits.
        Typical latency: < 0.1ms.

    Pass 2 (CLIP + Euclidean): semantic re-ranking of shortlist.
        Re-scores candidates by semantic similarity using neural embeddings.
        Euclidean distance on normalized vectors is equivalent to cosine ranking.
        Typical latency: < 0.5ms (search only, not encoding).

    Returns a dict with timings and results for each pass.
    """
    # -- Pass 1: Hamming shortlist --
    query_hash = dhash_vector(query_path)
    t0 = time.time()
    candidates = hash_col.search(vector=query_hash, top_k=shortlist_k)
    pass1_ms = (time.time() - t0) * 1000

    # -- Pass 2: CLIP re-ranking --
    query_clip = clip_embedding(query_path, model, preprocess)
    t0 = time.time()
    candidate_ids = {c["id"] for c in candidates}
    clip_results = clip_col.search(vector=query_clip, top_k=shortlist_k * 2)
    clip_scores = {r["id"]: r["score"] for r in clip_results}

    reranked = []
    for c in candidates:
        reranked.append({
            "id": c["id"],
            "filename": c["payload"]["filename"],
            "hamming_distance": c["score"],
            "clip_distance": clip_scores.get(c["id"], float("inf")),
        })
    reranked.sort(key=lambda x: x["clip_distance"])
    pass2_ms = (time.time() - t0) * 1000

    return {
        "query": os.path.basename(query_path),
        "pass1_ms": pass1_ms,
        "pass2_ms": pass2_ms,
        "total_ms": pass1_ms + pass2_ms,
        "hamming_results": [
            {"filename": c["payload"]["filename"], "distance": c["score"]}
            for c in candidates[:final_k]
        ],
        "combined_results": reranked[:final_k],
    }


# ---------------------------------------------------------------------------
# HTML Dashboard
# ---------------------------------------------------------------------------

def generate_html_report(results: dict, photo_dir: str, output_path: str):
    """Generate a self-contained HTML dashboard showing search results."""

    def img_to_base64(path: str, size: int = 150) -> str:
        img = Image.open(path).convert("RGB")
        img.thumbnail((size, size))
        buf = BytesIO()
        img.save(buf, format="JPEG", quality=80)
        return base64.b64encode(buf.getvalue()).decode()

    cards_html = ""
    for r in results["combined_results"]:
        img_path = os.path.join(photo_dir, r["filename"])
        if os.path.exists(img_path):
            b64 = img_to_base64(img_path)
            cards_html += f"""
            <div style="display:inline-block;margin:8px;text-align:center;
                        background:#1e1e2e;border-radius:8px;padding:12px;">
                <img src="data:image/jpeg;base64,{b64}"
                     style="border-radius:6px;max-width:150px;" /><br/>
                <b style="color:#cdd6f4;">{r['filename']}</b><br/>
                <span style="color:#89b4fa;">Hamming: {r['hamming_distance']}</span><br/>
                <span style="color:#a6e3a1;">CLIP L2: {r['clip_distance']:.4f}</span>
            </div>"""

    html = f"""<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <title>VelesDB Image Search Results</title>
    <style>
        body {{ background:#11111b; color:#cdd6f4; font-family:system-ui; padding:2rem; }}
        h1 {{ color:#89b4fa; }}
        .badge {{ display:inline-block; padding:4px 12px; border-radius:12px;
                  font-size:0.85rem; margin:4px; }}
        .fast {{ background:#a6e3a1; color:#11111b; }}
        .info {{ background:#89b4fa; color:#11111b; }}
    </style>
</head>
<body>
    <h1>Two-Pass Image Search Results</h1>
    <p>Query: <b>{results['query']}</b></p>
    <span class="badge fast">Pass 1 (Hamming): {results['pass1_ms']:.2f}ms</span>
    <span class="badge fast">Pass 2 (CLIP): {results['pass2_ms']:.2f}ms</span>
    <span class="badge info">Total: {results['total_ms']:.2f}ms</span>
    <h2 style="color:#f9e2af;">Combined Results (re-ranked by CLIP)</h2>
    {cards_html}
    <hr style="border-color:#313244;margin-top:2rem;"/>
    <p style="color:#6c7086;font-size:0.8rem;">
        Powered by <a href="https://velesdb.com" style="color:#89b4fa;">VelesDB</a>
        - Source-available vector+graph+columnar database
    </p>
</body>
</html>"""

    with open(output_path, "w") as f:
        f.write(html)
    print(f"  Dashboard saved to {output_path}")


# ---------------------------------------------------------------------------
# Demo: download sample images and run the full pipeline
# ---------------------------------------------------------------------------

def download_demo_images(photo_dir: str):
    """Download sample images from picsum.photos for demonstration."""
    os.makedirs(photo_dir, exist_ok=True)
    samples = [
        ("beach_1.jpg", "https://picsum.photos/seed/beach1/640/480"),
        ("beach_2.jpg", "https://picsum.photos/seed/ocean2/640/480"),
        ("mountain_1.jpg", "https://picsum.photos/seed/mount1/640/480"),
        ("city_1.jpg", "https://picsum.photos/seed/city11/640/480"),
        ("food_1.jpg", "https://picsum.photos/seed/food01/640/480"),
        ("flowers_1.jpg", "https://picsum.photos/seed/flower7/640/480"),
        ("tech_1.jpg", "https://picsum.photos/seed/tech01/640/480"),
        ("animal_1.jpg", "https://picsum.photos/seed/animal1/640/480"),
        # Near-duplicates of beach_1 (different sizes/crops)
        ("beach_1_small.jpg", "https://picsum.photos/seed/beach1/320/240"),
        ("beach_1_square.jpg", "https://picsum.photos/seed/beach1/480/480"),
    ]
    for name, url in samples:
        path = os.path.join(photo_dir, name)
        if not os.path.exists(path):
            print(f"  Downloading {name}...")
            urllib.request.urlretrieve(url, path)


def main():
    print("\n" + "=" * 60)
    print("VelesDB Two-Pass Image Search Pipeline")
    print("=" * 60)

    # Use demo images or point PHOTO_DIR to your own photos
    if not os.path.exists(PHOTO_DIR) or not os.listdir(PHOTO_DIR):
        print("\nDownloading demo images...")
        download_demo_images(PHOTO_DIR)

    # Clean previous DB
    shutil.rmtree(DB_DIR, ignore_errors=True)
    db = velesdb.Database(DB_DIR)

    # Step 1: Index perceptual hashes
    print("\nStep 1: Indexing perceptual hashes (dHash + Hamming)")
    hash_col, files = index_hashes(db, PHOTO_DIR)

    # Step 2: Index CLIP embeddings
    print("\nStep 2: Indexing CLIP embeddings (ViT-B-32 + Euclidean)")
    model, preprocess = load_clip_model()
    clip_col = index_clip(db, PHOTO_DIR, files, model, preprocess)

    # Step 3: Two-pass search
    query = os.path.join(PHOTO_DIR, "beach_1.jpg")
    print(f"\nStep 3: Two-pass search for '{os.path.basename(query)}'")
    results = two_pass_search(query, hash_col, clip_col, model, preprocess)

    # Display results
    print(f"\n  Pass 1 (Hamming shortlist): {results['pass1_ms']:.2f}ms")
    for r in results["hamming_results"]:
        print(f"    {r['filename']:25s}  distance={r['distance']}")

    print(f"\n  Pass 2 (CLIP re-ranked):    {results['pass2_ms']:.2f}ms")
    for r in results["combined_results"]:
        print(
            f"    {r['filename']:25s}  "
            f"hamming={r['hamming_distance']}  "
            f"clip_l2={r['clip_distance']:.4f}"
        )

    print(f"\n  Total pipeline: {results['total_ms']:.2f}ms")

    # Generate HTML dashboard
    print("\nGenerating HTML dashboard...")
    html_path = os.path.join(os.path.dirname(DB_DIR), "image_search_results.html")
    generate_html_report(results, PHOTO_DIR, html_path)

    print("\n" + "=" * 60)
    print("Done! Open the HTML file in your browser to see visual results.")
    print("=" * 60 + "\n")


if __name__ == "__main__":
    main()
