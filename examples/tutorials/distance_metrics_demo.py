"""
VelesDB Distance Metrics Tutorial - Complete Working Script
============================================================
5 distance metrics, 5 real-world use cases.

This script accompanies the article:
"Beyond Cosine: 5 Distance Metrics That Make Your Vector Database Surprisingly Versatile"

Links:
  Dev.to:    https://dev.to/wiscale
  Hashnode:  https://hashnode.com/@cyberlifecoder
  GitHub:    https://github.com/cyberlife-coder/VelesDB
  Docs:      https://velesdb.com/en/

Author: Julien Lange (@cyberlife-coder)
License: Elastic License 2.0 (source-available)
"""

import velesdb
import os
import shutil

DB_PATH = "./distance_metrics_demo"


def clean_db():
    if os.path.exists(DB_PATH):
        shutil.rmtree(DB_PATH)


def demo_cosine():
    """
    COSINE SIMILARITY - Semantic Search

    Cosine measures the angle between two vectors, ignoring their length.
    Two documents about the same topic will point in the same direction,
    even if one is longer than the other.

    Score: 1.0 = identical direction, 0.0 = unrelated, -1.0 = opposite
    """
    print("\n" + "=" * 60)
    print("1. COSINE - Semantic Search")
    print("   'Are these two things about the same topic?'")
    print("=" * 60)

    db = velesdb.Database(DB_PATH)
    collection = db.create_collection("articles", dimension=4, metric="cosine")

    # Simulated embeddings: [tech, science, cooking, sports]
    collection.upsert([
        {"id": 1, "vector": [0.9, 0.8, 0.0, 0.0], "payload": {"title": "Introduction to Machine Learning"}},
        {"id": 2, "vector": [0.8, 0.9, 0.0, 0.0], "payload": {"title": "Neural Networks Explained"}},
        {"id": 3, "vector": [0.0, 0.1, 0.9, 0.8], "payload": {"title": "Best Pasta Recipes"}},
        {"id": 4, "vector": [0.1, 0.0, 0.0, 0.9], "payload": {"title": "World Cup 2026 Preview"}},
        {"id": 5, "vector": [0.7, 0.6, 0.0, 0.1], "payload": {"title": "Deep Learning for Beginners"}},
    ])

    query = [0.85, 0.75, 0.0, 0.0]
    results = collection.search_request(velesdb.SearchOptions(vector=query, top_k=5))

    print('\nQuery: "I want to learn about AI"')
    print("Cosine finds articles pointing in the same direction:\n")
    for r in results:
        bar = "#" * int(r["score"] * 20)
        print(f"  {r['score']:.3f} {bar:20s} {r['payload']['title']}")

    print("\n  -> Cosine ignores magnitude. A short tweet and a long paper")
    print("     about AI would score equally high.")
    return True


def demo_euclidean():
    """
    EUCLIDEAN DISTANCE - Anomaly Detection

    Euclidean measures the straight-line distance between two points.
    Think of it as measuring with a ruler on a map.
    Close points = similar, far points = different.

    Key insight: unlike cosine, euclidean CARES about magnitude.
    """
    print("\n" + "=" * 60)
    print("2. EUCLIDEAN - Anomaly Detection (IoT Sensors)")
    print("   'Is this reading normal or abnormal?'")
    print("=" * 60)

    db = velesdb.Database(DB_PATH)
    collection = db.create_collection("sensors", dimension=4, metric="euclidean")

    # Sensor readings: [temperature_C, pressure_hPa, humidity_%, vibration_g]
    collection.upsert([
        {"id": 1, "vector": [22.0, 1013.0, 45.0, 0.3], "payload": {"status": "normal", "time": "08:00"}},
        {"id": 2, "vector": [22.5, 1012.5, 47.0, 0.3], "payload": {"status": "normal", "time": "09:00"}},
        {"id": 3, "vector": [21.8, 1013.5, 44.0, 0.4], "payload": {"status": "normal", "time": "10:00"}},
        {"id": 4, "vector": [23.0, 1012.0, 46.0, 0.3], "payload": {"status": "normal", "time": "11:00"}},
        {"id": 5, "vector": [78.0, 985.0, 12.0, 4.8], "payload": {"status": "ANOMALY", "time": "11:15"}},
    ])

    baseline = [22.0, 1013.0, 45.0, 0.3]
    results = collection.search_request(velesdb.SearchOptions(vector=baseline, top_k=5))

    print("\nBaseline: [22C, 1013hPa, 45%, 0.3g]")
    print("Finding closest readings to the baseline:\n")
    for r in results:
        dist = r["score"]
        flag = " *** ALERT ***" if dist > 10 else ""
        print(f"  distance={dist:8.2f}  {r['payload']['time']} - {r['payload']['status']}{flag}")

    print("\n  -> The anomaly is 100x farther than normal readings.")
    print("     Cosine would MISS this because both vectors could point")
    print("     in a similar direction. Euclidean catches the magnitude shift.")
    return True


def demo_dotproduct():
    """
    DOT PRODUCT - Recommendation Ranking

    Dot product = cosine similarity * magnitude of both vectors.
    It captures both direction (relevance) AND magnitude (confidence/quality).
    """
    print("\n" + "=" * 60)
    print("3. DOT PRODUCT - Smart Recommendations")
    print("   'What should I watch, ranked by relevance AND quality?'")
    print("=" * 60)

    db = velesdb.Database(DB_PATH)
    collection = db.create_collection("movies", dimension=4, metric="dotproduct")

    # Embedding dimensions: [sci_fi, action, drama, comedy]
    collection.upsert([
        {"id": 1, "vector": [0.95, 0.80, 0.30, 0.05], "payload": {"title": "Interstellar", "rating": 8.7}},
        {"id": 2, "vector": [0.40, 0.35, 0.10, 0.02], "payload": {"title": "Low-Budget Sci-Fi B-Movie", "rating": 3.2}},
        {"id": 3, "vector": [0.85, 0.90, 0.20, 0.10], "payload": {"title": "The Matrix", "rating": 8.7}},
        {"id": 4, "vector": [0.05, 0.05, 0.10, 0.95], "payload": {"title": "Comedy Special", "rating": 7.0}},
        {"id": 5, "vector": [0.70, 0.60, 0.50, 0.05], "payload": {"title": "Blade Runner 2049", "rating": 8.0}},
    ])

    user = [0.9, 0.8, 0.1, 0.0]
    results = collection.search_request(velesdb.SearchOptions(vector=user, top_k=5))

    print('\nUser profile: loves sci-fi + action')
    print("Dot product ranks by relevance AND confidence:\n")
    for r in results:
        bar = "#" * int(r["score"] * 10)
        print(f"  score={r['score']:.3f} {bar:15s} {r['payload']['title']} (rating: {r['payload']['rating']})")

    print("\n  -> 'Low-Budget Sci-Fi' points in the same direction as 'Interstellar'")
    print("     but dot product ranks it lower because of weaker magnitude.")
    print("     Cosine would rank them equally!")
    return True


def demo_hamming():
    """
    HAMMING DISTANCE - Image Deduplication

    Hamming counts how many bits differ between two binary vectors.
    Perfect for comparing hashes: perceptual hashes of images,
    fingerprints of audio, binary feature flags.

    Distance 0 = identical, 1 = one bit different, etc.
    """
    print("\n" + "=" * 60)
    print("4. HAMMING - Image Deduplication")
    print("   'Is this image a copy of one we already have?'")
    print("=" * 60)

    db = velesdb.Database(DB_PATH)
    collection = db.create_collection("image_hashes", dimension=16, metric="hamming")

    # Simulated 16-bit perceptual hashes (pHash)
    collection.upsert([
        {"id": 1, "vector": [1, 0, 1, 1, 0, 0, 1, 0, 1, 1, 0, 1, 0, 0, 1, 1],
         "payload": {"file": "sunset_original.jpg", "source": "photographer"}},
        {"id": 2, "vector": [1, 0, 1, 1, 0, 0, 1, 0, 1, 1, 0, 1, 0, 0, 1, 0],
         "payload": {"file": "sunset_cropped.jpg", "source": "instagram repost"}},
        {"id": 3, "vector": [1, 0, 1, 1, 0, 0, 1, 0, 1, 1, 0, 1, 0, 1, 1, 1],
         "payload": {"file": "sunset_filtered.jpg", "source": "pinterest"}},
        {"id": 4, "vector": [0, 1, 0, 0, 1, 1, 0, 1, 0, 0, 1, 0, 1, 1, 0, 0],
         "payload": {"file": "cat_photo.jpg", "source": "original"}},
        {"id": 5, "vector": [1, 0, 1, 1, 0, 0, 1, 0, 1, 1, 0, 1, 0, 0, 0, 1],
         "payload": {"file": "sunset_watermarked.jpg", "source": "stock site"}},
    ])

    new_upload = [1, 0, 1, 1, 0, 0, 1, 0, 1, 1, 0, 1, 0, 0, 1, 1]
    results = collection.search_request(velesdb.SearchOptions(vector=new_upload, top_k=5))

    print("\nNew image uploaded. Checking for duplicates...\n")
    for r in results:
        dist = int(r["score"])
        if dist <= 2:
            status = "DUPLICATE" if dist == 0 else "NEAR-DUPLICATE"
        else:
            status = "different image"
        print(f"  hamming={dist:2d} bits  [{status:15s}]  {r['payload']['file']} ({r['payload']['source']})")

    print("\n  -> Hamming distance <= 2 bits? Almost certainly a copy.")
    print("     Catches crops, filters, watermarks, and recompression.")
    return True


def demo_jaccard():
    """
    JACCARD SIMILARITY - Taste Matching / Set Overlap

    Jaccard = size of intersection / size of union.
    Perfect for comparing sets: what genres do two users share?

    Score: 1.0 = identical sets, 0.0 = no overlap
    """
    print("\n" + "=" * 60)
    print("5. JACCARD - Taste Matching")
    print("   'Which users share the most interests with me?'")
    print("=" * 60)

    db = velesdb.Database(DB_PATH)
    collection = db.create_collection("user_profiles", dimension=10, metric="jaccard")

    # Genres: [action, comedy, sci-fi, horror, drama, romance, thriller, documentary, anime, musical]
    collection.upsert([
        {"id": 1, "vector": [1, 0, 1, 0, 1, 0, 1, 0, 0, 0],
         "payload": {"user": "Alice", "likes": "action, sci-fi, drama, thriller"}},
        {"id": 2, "vector": [0, 1, 0, 0, 0, 1, 0, 0, 0, 1],
         "payload": {"user": "Bob", "likes": "comedy, romance, musical"}},
        {"id": 3, "vector": [1, 0, 1, 1, 0, 0, 1, 0, 1, 0],
         "payload": {"user": "Charlie", "likes": "action, sci-fi, horror, thriller, anime"}},
        {"id": 4, "vector": [1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
         "payload": {"user": "Dave", "likes": "literally everything"}},
        {"id": 5, "vector": [1, 0, 1, 0, 0, 0, 0, 1, 0, 0],
         "payload": {"user": "Eve", "likes": "action, sci-fi, documentary"}},
    ])

    my_tastes = [1, 0, 1, 0, 1, 0, 0, 0, 0, 0]
    results = collection.search_request(velesdb.SearchOptions(vector=my_tastes, top_k=5))

    print("\nYou like: action, sci-fi, drama")
    print("Finding users with the most taste overlap:\n")
    for r in results:
        pct = r["score"] * 100
        bar = "#" * int(pct / 5)
        print(f"  {pct:5.1f}% match {bar:20s} {r['payload']['user']} ({r['payload']['likes']})")

    print("\n  -> Alice shares 3/4 of her genres with you = 75% Jaccard match.")
    print("     Dave likes everything but his union is huge, so only 30%.")
    print("     No ML model needed. No embeddings. Just set math.")
    return True


def main():
    clean_db()

    print("VelesDB Distance Metrics Demo")
    print("5 metrics, 5 real-world use cases")
    print("=" * 60)

    all_passed = True
    all_passed &= demo_cosine()
    all_passed &= demo_euclidean()
    all_passed &= demo_dotproduct()
    all_passed &= demo_hamming()
    all_passed &= demo_jaccard()

    print("\n" + "=" * 60)
    if all_passed:
        print("ALL 5 DEMOS COMPLETED SUCCESSFULLY")
    else:
        print("SOME DEMOS FAILED")
    print("=" * 60)

    clean_db()
    return all_passed


if __name__ == "__main__":
    success = main()
    exit(0 if success else 1)
