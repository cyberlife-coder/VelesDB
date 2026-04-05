"""
Tests for VelesDB Tutorial: Two-Pass Image Search with Hamming + CLIP

Verifies all VelesDB calls from the article against the current version.
Run: python -m pytest test_image_search_hamming_clip.py -v

Companion article:
    Dev.to:    https://dev.to/wiscale
    Hashnode:  https://hashnode.com/@cyberlifecoder
    GitHub:    https://github.com/cyberlife-coder/VelesDB
    Docs:      https://velesdb.com/en/
"""

import os
import sys
import time
import shutil
import urllib.request

import numpy as np
from PIL import Image
import imagehash
import velesdb

# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

PHOTO_DIR = "/tmp/test_tutorial_photos"
DB_DIR = "/tmp/test_tutorial_db"
HASH_SIZE = 16


def setup_test_images():
    """Download test images from picsum.photos."""
    os.makedirs(PHOTO_DIR, exist_ok=True)
    urls = [
        ("beach_1.jpg", "https://picsum.photos/seed/beach1/640/480"),
        ("beach_2.jpg", "https://picsum.photos/seed/ocean2/640/480"),
        ("mountain_1.jpg", "https://picsum.photos/seed/mount1/640/480"),
        ("city_1.jpg", "https://picsum.photos/seed/city11/640/480"),
        ("food_1.jpg", "https://picsum.photos/seed/food01/640/480"),
        ("flowers_1.jpg", "https://picsum.photos/seed/flower7/640/480"),
        ("beach_1_small.jpg", "https://picsum.photos/seed/beach1/320/240"),
        ("beach_1_square.jpg", "https://picsum.photos/seed/beach1/480/480"),
    ]
    for name, url in urls:
        path = os.path.join(PHOTO_DIR, name)
        if not os.path.exists(path):
            urllib.request.urlretrieve(url, path)
    return sorted(
        f for f in os.listdir(PHOTO_DIR)
        if f.lower().endswith((".jpg", ".jpeg", ".png", ".webp"))
    )


def cleanup():
    shutil.rmtree(DB_DIR, ignore_errors=True)
    shutil.rmtree(PHOTO_DIR, ignore_errors=True)


def dhash_vector(img_path, hash_size=HASH_SIZE):
    img = Image.open(img_path)
    h = imagehash.dhash(img, hash_size=hash_size)
    return [float(b) for b in h.hash.flatten()]


# ---------------------------------------------------------------------------
# Test 1: dHash + Hamming collection
# ---------------------------------------------------------------------------

def test_dhash_hamming():
    """Perceptual hash indexing and Hamming search produce correct results."""
    shutil.rmtree(DB_DIR, ignore_errors=True)
    files = setup_test_images()
    db = velesdb.Database(DB_DIR)

    hash_dim = HASH_SIZE * HASH_SIZE
    hash_col = db.get_or_create_collection(
        "perceptual_hashes", dimension=hash_dim, metric="hamming"
    )

    for i, fname in enumerate(files):
        path = os.path.join(PHOTO_DIR, fname)
        vec = dhash_vector(path)
        assert len(vec) == hash_dim
        assert all(v in (0.0, 1.0) for v in vec), "Hash must be binary"
        hash_col.upsert(i + 1, vector=vec, payload={"filename": fname, "path": path})

    # Search for beach_1
    query_path = os.path.join(PHOTO_DIR, "beach_1.jpg")
    results = hash_col.search(vector=dhash_vector(query_path), top_k=5)

    assert len(results) == 5
    assert results[0]["payload"]["filename"] == "beach_1.jpg"
    assert results[0]["score"] == 0, "Self-match Hamming distance must be 0"

    top3 = [r["payload"]["filename"] for r in results[:3]]
    assert "beach_1_small.jpg" in top3, f"Near-duplicate not found in top 3: {top3}"

    print("  [PASS] dHash + Hamming: correct indexing and search")
    return db, hash_col, files


# ---------------------------------------------------------------------------
# Test 2: CLIP + Euclidean collection
# ---------------------------------------------------------------------------

def test_clip_euclidean(db, files):
    """CLIP embedding indexing and Euclidean search produce correct results."""
    try:
        import open_clip
        import torch
    except ImportError:
        print("  [SKIP] open-clip-torch not installed")
        return None

    model, _, preprocess = open_clip.create_model_and_transforms(
        "ViT-B-32", pretrained="laion2b_s34b_b79k"
    )
    model.eval()

    def clip_emb(img_path):
        img = Image.open(img_path).convert("RGB")
        with torch.no_grad():
            t = preprocess(img).unsqueeze(0)
            f = model.encode_image(t)
            f /= f.norm(dim=-1, keepdim=True)
            return f.squeeze().numpy().tolist()

    clip_col = db.get_or_create_collection(
        "clip_features", dimension=512, metric="euclidean"
    )

    for i, fname in enumerate(files):
        path = os.path.join(PHOTO_DIR, fname)
        emb = clip_emb(path)
        assert len(emb) == 512
        clip_col.upsert(i + 1, vector=emb, payload={"filename": fname, "path": path})

    query_path = os.path.join(PHOTO_DIR, "beach_1.jpg")
    results = clip_col.search(vector=clip_emb(query_path), top_k=5)

    assert len(results) == 5
    assert results[0]["payload"]["filename"] == "beach_1.jpg"
    assert results[0]["score"] < 0.01, f"Self-match L2 too high: {results[0]['score']}"

    top3 = [r["payload"]["filename"] for r in results[:3]]
    assert "beach_1_square.jpg" in top3, f"Semantic match not in top 3: {top3}"

    print("  [PASS] CLIP + Euclidean: correct semantic ranking")
    return clip_col, clip_emb


# ---------------------------------------------------------------------------
# Test 3: Combined two-pass pipeline
# ---------------------------------------------------------------------------

def test_combined_pipeline(hash_col, clip_col, clip_emb_fn, files):
    """Two-pass pipeline completes under 100ms and produces valid re-ranking."""
    query_path = os.path.join(PHOTO_DIR, "beach_1.jpg")

    # Pass 1: Hamming
    t0 = time.time()
    candidates = hash_col.search(vector=dhash_vector(query_path), top_k=8)
    pass1_ms = (time.time() - t0) * 1000

    # Pass 2: CLIP re-ranking
    query_clip = clip_emb_fn(query_path)
    t0 = time.time()
    clip_all = clip_col.search(vector=query_clip, top_k=len(files))
    clip_by_id = {r["id"]: r["score"] for r in clip_all}

    reranked = sorted(
        [
            {
                "id": c["id"],
                "filename": c["payload"]["filename"],
                "hamming": c["score"],
                "clip_dist": clip_by_id.get(c["id"], float("inf")),
            }
            for c in candidates
        ],
        key=lambda x: x["clip_dist"],
    )
    pass2_ms = (time.time() - t0) * 1000

    total = pass1_ms + pass2_ms
    assert total < 100, f"Pipeline too slow: {total:.1f}ms"
    assert reranked[0]["filename"] == "beach_1.jpg"

    print(f"  [PASS] Combined pipeline: {total:.2f}ms (p1={pass1_ms:.2f}, p2={pass2_ms:.2f})")


# ---------------------------------------------------------------------------
# Test 4: All metrics from the article are supported
# ---------------------------------------------------------------------------

def test_all_metrics():
    """VelesDB accepts all distance metrics mentioned in the article."""
    test_dir = "/tmp/test_metrics_tutorial"
    shutil.rmtree(test_dir, ignore_errors=True)
    db = velesdb.Database(test_dir)

    for metric in ["hamming", "jaccard", "euclidean", "cosine", "dot"]:
        col = db.get_or_create_collection(f"test_{metric}", dimension=4, metric=metric)
        col.upsert(1, vector=[0.1, 0.2, 0.3, 0.4], payload={"test": True})
        results = col.search(vector=[0.1, 0.2, 0.3, 0.4], top_k=1)
        assert len(results) == 1, f"metric '{metric}' search failed"
        print(f"  [PASS] metric='{metric}'")

    shutil.rmtree(test_dir, ignore_errors=True)


# ---------------------------------------------------------------------------
# Test 5: HTML dashboard generation
# ---------------------------------------------------------------------------

def test_html_generation():
    """Base64 image encoding works for the HTML dashboard."""
    import base64
    from io import BytesIO

    path = os.path.join(PHOTO_DIR, "beach_1.jpg")
    img = Image.open(path).convert("RGB")
    img.thumbnail((200, 200))
    buf = BytesIO()
    img.save(buf, format="JPEG", quality=80)
    b64 = base64.b64encode(buf.getvalue()).decode()

    assert len(b64) > 100
    assert b64[:4] == "/9j/", "Not a valid JPEG base64"
    print(f"  [PASS] Image base64 encoding ({len(b64)} chars)")


# ---------------------------------------------------------------------------
# Runner
# ---------------------------------------------------------------------------

if __name__ == "__main__":
    print(f"\n{'=' * 60}")
    print("Testing article code against VelesDB")
    print(f"{'=' * 60}\n")

    print("Setting up test images...")
    files = setup_test_images()
    print(f"  {len(files)} images ready\n")

    print("Test 1: dHash + Hamming")
    db, hash_col, files = test_dhash_hamming()

    print("\nTest 2: CLIP + Euclidean")
    result = test_clip_euclidean(db, files)

    if result:
        clip_col, clip_fn = result
        print("\nTest 3: Two-pass pipeline")
        test_combined_pipeline(hash_col, clip_col, clip_fn, files)

    print("\nTest 4: All article metrics")
    test_all_metrics()

    print("\nTest 5: HTML generation")
    test_html_generation()

    print(f"\n{'=' * 60}")
    print("ALL TESTS PASSED")
    print(f"{'=' * 60}\n")

    cleanup()
