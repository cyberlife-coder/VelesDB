#!/usr/bin/env python3
"""
VelesDB Python Example

This example demonstrates how to use VelesDB with Python and the requests library.
For production use, consider using the official VelesDB Python SDK (Premium).
"""

import requests
import json
from typing import List, Dict, Any, Optional

# VelesDB server URL
VELESDB_URL = "http://localhost:8080"


class VelesDBClient:
    """Simple VelesDB client for Python."""

    def __init__(self, base_url: str = VELESDB_URL):
        self.base_url = base_url.rstrip("/")

    def health(self) -> Dict[str, Any]:
        """Check server health."""
        response = requests.get(f"{self.base_url}/health")
        response.raise_for_status()
        return response.json()

    def create_collection(
        self,
        name: str,
        dimension: int,
        metric: str = "cosine"
    ) -> Dict[str, Any]:
        """Create a new collection."""
        response = requests.post(
            f"{self.base_url}/collections",
            json={"name": name, "dimension": dimension, "metric": metric}
        )
        response.raise_for_status()
        return response.json()

    def list_collections(self) -> List[str]:
        """List all collections."""
        response = requests.get(f"{self.base_url}/collections")
        response.raise_for_status()
        return response.json()["collections"]

    def delete_collection(self, name: str) -> Dict[str, Any]:
        """Delete a collection."""
        response = requests.delete(f"{self.base_url}/collections/{name}")
        response.raise_for_status()
        return response.json()

    def upsert(
        self,
        collection: str,
        points: List[Dict[str, Any]]
    ) -> Dict[str, Any]:
        """Insert or update points."""
        response = requests.post(
            f"{self.base_url}/collections/{collection}/points",
            json={"points": points}
        )
        response.raise_for_status()
        return response.json()

    def search(
        self,
        collection: str,
        vector: List[float],
        top_k: int = 10
    ) -> List[Dict[str, Any]]:
        """Search for similar vectors."""
        response = requests.post(
            f"{self.base_url}/collections/{collection}/search",
            json={"vector": vector, "top_k": top_k}
        )
        response.raise_for_status()
        return response.json()["results"]


def main():
    """Example usage of VelesDB."""
    client = VelesDBClient()

    # Check health
    print("Checking server health...")
    health = client.health()
    print(f"Server status: {health['status']}, version: {health['version']}")

    # Create a collection
    print("\nCreating collection 'documents'...")
    try:
        client.create_collection("documents", dimension=4, metric="cosine")
        print("Collection created!")
    except requests.HTTPError as e:
        if e.response.status_code == 400:
            print("Collection already exists, continuing...")
        else:
            raise

    # Insert some vectors
    print("\nInserting vectors...")
    points = [
        {
            "id": 1,
            "vector": [1.0, 0.0, 0.0, 0.0],
            "payload": {"title": "Document A", "category": "tech"}
        },
        {
            "id": 2,
            "vector": [0.0, 1.0, 0.0, 0.0],
            "payload": {"title": "Document B", "category": "science"}
        },
        {
            "id": 3,
            "vector": [0.0, 0.0, 1.0, 0.0],
            "payload": {"title": "Document C", "category": "tech"}
        },
        {
            "id": 4,
            "vector": [0.9, 0.1, 0.0, 0.0],
            "payload": {"title": "Document D", "category": "tech"}
        },
    ]
    result = client.upsert("documents", points)
    print(f"Inserted {result['count']} points")

    # Search for similar vectors
    print("\nSearching for vectors similar to [1.0, 0.0, 0.0, 0.0]...")
    query = [1.0, 0.0, 0.0, 0.0]
    results = client.search("documents", query, top_k=3)

    print("\nSearch results:")
    for i, result in enumerate(results, 1):
        print(f"  {i}. ID: {result['id']}, Score: {result['score']:.4f}")
        if result.get('payload'):
            print(f"     Title: {result['payload'].get('title')}")

    # List collections
    print("\nCollections:", client.list_collections())


if __name__ == "__main__":
    main()
