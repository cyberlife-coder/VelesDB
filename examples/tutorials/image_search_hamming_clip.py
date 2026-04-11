"""
VelesDB Tutorial: Two-Pass Image Search with Hamming + CLIP

The Bouncer and the Detective:
  Pass 1 - The Bouncer (dHash barcodes + Hamming distance) for fast shortlisting
  Pass 2 - The Detective (CLIP meaning + Cosine similarity) for semantic re-ranking

Find duplicate and similar photos in 0.3ms using a combined pipeline.

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

from PIL import Image
import imagehash
import velesdb

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

PHOTO_DIR = os.environ.get("PHOTO_DIR", "./demo_photos")
DB_DIR = os.environ.get("DB_DIR", "./velesdb_image_search")
HASH_SIZE = 16  # dHash dimension: 16x16 = 256-bit barcode
CLIP_DIM = 512  # ViT-B-32 output dimension
SHORTLIST_K = 10  # Pass 1 candidates
FINAL_K = 5  # Final results


# ---------------------------------------------------------------------------
# Step 1: The Bouncer - perceptual hashing (dHash) as binary barcodes
# ---------------------------------------------------------------------------

def compute_barcode(img_path: str, hash_size: int = HASH_SIZE) -> list[float]:
    """Turn an image into a binary barcode for the Bouncer.

    dHash (difference hash) works by:
      1. Shrinking the image to a (hash_size+1) x hash_size grayscale grid
      2. Comparing each square to its right neighbor: darker = 1, else 0
      3. Producing a 256-bit barcode: [1, 0, 1, 1, 0, 0, 1, ...]

    The barcode barely changes when you resize, crop slightly, or compress.
    """
    img = Image.open(img_path)
    h = imagehash.dhash(img, hash_size=hash_size)
    return [float(b) for b in h.hash.flatten()]


def index_barcodes(db: velesdb.Database, photo_dir: str) -> tuple:
    """Index all images with dHash barcodes into the Bouncer's Hamming collection."""
    barcode_dim = HASH_SIZE * HASH_SIZE  # 256
    bouncer = db.get_or_create_collection(
        "perceptual_hashes", dimension=barcode_dim, metric="hamming"
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
        vec = compute_barcode(path)
        bouncer.upsert(i + 1, vector=vec, payload={"filename": fname, "path": path})
    elapsed_ms = (time.time() - t0) * 1000

    print(f"  Indexed {len(files)} barcodes in {elapsed_ms:.1f}ms")
    return bouncer, files


# ---------------------------------------------------------------------------
# Step 2: The Detective - CLIP embeddings as a map of meaning
# ---------------------------------------------------------------------------

def load_clip_model():
    """Load CLIP ViT-B-32 model for the Detective's semantic map."""
    try:
        # pylint: disable=import-outside-toplevel,unused-import
        import open_clip
        import torch  # noqa: F401  # availability probe — used via `torch.no_grad()` in compute_meaning
    except ImportError:
        print("open-clip-torch is required for CLIP embeddings.")
        print("Install: pip install open-clip-torch torch")
        sys.exit(1)

    model, _, preprocess = open_clip.create_model_and_transforms(
        "ViT-B-32", pretrained="laion2b_s34b_b79k"
    )
    model.eval()
    return model, preprocess


def compute_meaning(img_path: str, model, preprocess) -> list[float]:
    """Place an image on the Detective's map of meaning.

    CLIP (Contrastive Language-Image Pre-training) encodes images and text
    into a shared 512-dimensional semantic space. Two images with similar
    meaning will be close together, even if their pixels are completely different.
    """
    import torch

    img = Image.open(img_path).convert("RGB")
    with torch.no_grad():
        tensor = preprocess(img).unsqueeze(0)
        features = model.encode_image(tensor)
        features /= features.norm(dim=-1, keepdim=True)
        return features.squeeze().numpy().tolist()


def index_meanings(db: velesdb.Database, photo_dir: str, files: list, model, preprocess) -> tuple:
    """Index all images with CLIP embeddings into the Detective's Cosine collection."""
    detective = db.get_or_create_collection(
        "clip_features", dimension=CLIP_DIM, metric="cosine"
    )

    t0 = time.time()
    for i, fname in enumerate(files):
        path = os.path.join(photo_dir, fname)
        emb = compute_meaning(path, model, preprocess)
        detective.upsert(i + 1, vector=emb, payload={"filename": fname, "path": path})
    elapsed = time.time() - t0

    print(f"  Indexed {len(files)} meanings in {elapsed:.1f}s")
    return detective


# ---------------------------------------------------------------------------
# Step 3: Two-pass search - Bouncer filters, Detective re-ranks
# ---------------------------------------------------------------------------

def find_similar(
    query_path: str,
    bouncer,
    detective,
    model,
    preprocess,
    shortlist_k: int = SHORTLIST_K,
    final_k: int = FINAL_K,
) -> dict:
    """Two-pass search: Bouncer filters, Detective re-ranks.

    Pass 1 (Hamming): The Bouncer looks at barcodes for one second.
        Catches obvious fakes instantly. Ultra-fast (< 0.1ms).

    Pass 2 (CLIP + Cosine): The Detective runs a thorough investigation.
        ONE single CLIP query, then joins scores. Not N queries.
        This is what makes it scale.

    Returns a dict with timings and results for each pass.
    """
    # --- Pass 1: The Bouncer (instant) ---
    query_barcode = compute_barcode(query_path)
    t0 = time.time()
    fast_candidates = bouncer.search(vector=query_barcode, top_k=shortlist_k)
    bouncer_ms = (time.time() - t0) * 1000

    # --- Pass 2: The Detective (thorough) ---
    # ONE single CLIP query. Not N. This is what makes it scale.
    query_meaning = compute_meaning(query_path, model, preprocess)
    t0 = time.time()
    all_meanings = detective.search(vector=query_meaning, top_k=shortlist_k * 2)
    meaning_scores = {r["id"]: r["score"] for r in all_meanings}

    # Re-rank the Bouncer's shortlist with the Detective's scores
    reranked = []
    for c in fast_candidates:
        reranked.append({
            "id": c["id"],
            "filename": c["payload"]["filename"],
            "bouncer": c["score"],
            "detective": meaning_scores.get(c["id"], 0.0),
        })
    reranked.sort(key=lambda x: x["detective"], reverse=True)
    detective_ms = (time.time() - t0) * 1000

    return {
        "query": os.path.basename(query_path),
        "bouncer_ms": bouncer_ms,
        "detective_ms": detective_ms,
        "total_ms": bouncer_ms + detective_ms,
        "bouncer_results": [
            {"filename": c["payload"]["filename"], "distance": c["score"]}
            for c in fast_candidates[:final_k]
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
                <span style="color:#89b4fa;">Bouncer: {r['bouncer']}</span><br/>
                <span style="color:#a6e3a1;">Detective: {r['detective']:.4f}</span>
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
    <span class="badge fast">Bouncer (Hamming): {results['bouncer_ms']:.2f}ms</span>
    <span class="badge fast">Detective (CLIP): {results['detective_ms']:.2f}ms</span>
    <span class="badge info">Total: {results['total_ms']:.2f}ms</span>
    <h2 style="color:#f9e2af;">Combined Results (re-ranked by Detective)</h2>
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
    print("VelesDB Two-Pass Image Search: The Bouncer and the Detective")
    print("=" * 60)

    # Use demo images or point PHOTO_DIR to your own photos
    if not os.path.exists(PHOTO_DIR) or not os.listdir(PHOTO_DIR):
        print("\nDownloading demo images...")
        download_demo_images(PHOTO_DIR)

    # Clean previous DB
    shutil.rmtree(DB_DIR, ignore_errors=True)
    db = velesdb.Database(DB_DIR)

    # Step 1: Give the Bouncer his barcodes
    print("\nStep 1: The Bouncer indexes barcodes (dHash + Hamming)")
    bouncer, files = index_barcodes(db, PHOTO_DIR)

    # Step 2: Give the Detective his map
    print("\nStep 2: The Detective builds his map of meaning (CLIP + Cosine)")
    model, preprocess = load_clip_model()
    detective = index_meanings(db, PHOTO_DIR, files, model, preprocess)

    # Step 3: Two-pass search
    query = os.path.join(PHOTO_DIR, "beach_1.jpg")
    print(f"\nStep 3: Two-pass search for '{os.path.basename(query)}'")
    results = find_similar(query, bouncer, detective, model, preprocess)

    # Display results
    print(f"\n  Bouncer (Hamming shortlist): {results['bouncer_ms']:.2f}ms")
    for r in results["bouncer_results"]:
        print(f"    {r['filename']:25s}  distance={r['distance']}")

    print(f"\n  Detective (CLIP re-ranked):  {results['detective_ms']:.2f}ms")
    for r in results["combined_results"]:
        print(
            f"    {r['filename']:25s}  "
            f"bouncer={r['bouncer']}  "
            f"detective={r['detective']:.4f}"
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
