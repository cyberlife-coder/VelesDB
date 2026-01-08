#!/usr/bin/env python3
"""
VelesDB Python Example

This example demonstrates how to use VelesDB with Python and the requests library.
For production use, consider using the official VelesDB Python SDK (Premium).
"""

import logging
import requests
from requests.adapters import HTTPAdapter
from urllib3.util.retry import Retry
from typing import List, Dict, Any

# Configure logging
logging.basicConfig(level=logging.INFO, format='%(asctime)s - %(levelname)s - %(message)s')
logger = logging.getLogger(__name__)

# VelesDB server URL
VELESDB_URL = "http://localhost:8080"

# Request timeout in seconds
DEFAULT_TIMEOUT = 30


class VelesDBClient:
    """Simple VelesDB client for Python.
    
    Args:
        base_url: VelesDB server URL (default: http://localhost:8080)
        timeout: Request timeout in seconds (default: 30)
        retries: Number of retry attempts for failed requests (default: 3)
    """

    def __init__(
        self,
        base_url: str = VELESDB_URL,
        timeout: float = DEFAULT_TIMEOUT,
        retries: int = 3
    ):
        self.base_url = base_url.rstrip("/")
        self.timeout = timeout
        
        # Configure session with retry logic
        self.session = requests.Session()
        retry_strategy = Retry(
            total=retries,
            backoff_factor=0.5,
            status_forcelist=[500, 502, 503, 504],
        )
        adapter = HTTPAdapter(max_retries=retry_strategy)
        self.session.mount("http://", adapter)
        self.session.mount("https://", adapter)

    def health(self) -> Dict[str, Any]:
        """Check server health.
        
        Returns:
            Dict with 'status' and 'version' keys.
            
        Raises:
            requests.RequestException: If server is unreachable.
        """
        response = self.session.get(f"{self.base_url}/health", timeout=self.timeout)
        response.raise_for_status()
        return response.json()

    def create_collection(
        self,
        name: str,
        dimension: int,
        metric: str = "cosine"
    ) -> Dict[str, Any]:
        """Create a new collection.
        
        Args:
            name: Collection name (alphanumeric, underscores allowed).
            dimension: Vector dimension (must match your embeddings).
            metric: Distance metric ('cosine', 'euclidean', 'dot').
            
        Returns:
            Dict with creation confirmation.
        """
        response = self.session.post(
            f"{self.base_url}/collections",
            json={"name": name, "dimension": dimension, "metric": metric},
            timeout=self.timeout
        )
        response.raise_for_status()
        return response.json()

    def list_collections(self) -> List[str]:
        """List all collections.
        
        Returns:
            List of collection names.
        """
        response = self.session.get(f"{self.base_url}/collections", timeout=self.timeout)
        response.raise_for_status()
        return response.json()["collections"]

    def delete_collection(self, name: str) -> Dict[str, Any]:
        """Delete a collection.
        
        Args:
            name: Collection name to delete.
            
        Returns:
            Dict with deletion confirmation.
        """
        response = self.session.delete(
            f"{self.base_url}/collections/{name}",
            timeout=self.timeout
        )
        response.raise_for_status()
        return response.json()

    def upsert(
        self,
        collection: str,
        points: List[Dict[str, Any]]
    ) -> Dict[str, Any]:
        """Insert or update points (vectors with optional payload).
        
        Args:
            collection: Target collection name.
            points: List of point dicts with 'id', 'vector', and optional 'payload'.
            
        Returns:
            Dict with 'count' of upserted points.
        """
        response = self.session.post(
            f"{self.base_url}/collections/{collection}/points",
            json={"points": points},
            timeout=self.timeout
        )
        response.raise_for_status()
        return response.json()

    def search(
        self,
        collection: str,
        vector: List[float],
        top_k: int = 10
    ) -> List[Dict[str, Any]]:
        """Search for similar vectors.
        
        Args:
            collection: Collection to search in.
            vector: Query vector (must match collection dimension).
            top_k: Number of results to return (default: 10).
            
        Returns:
            List of results with 'id', 'score', and 'payload'.
        """
        response = self.session.post(
            f"{self.base_url}/collections/{collection}/search",
            json={"vector": vector, "top_k": top_k},
            timeout=self.timeout
        )
        response.raise_for_status()
        return response.json()["results"]


def main():
    """Example usage of VelesDB."""
    client = VelesDBClient()

    # Check health
    logger.info("Checking server health...")
    try:
        health = client.health()
        logger.info(f"Server status: {health['status']}, version: {health['version']}")
    except requests.ConnectionError:
        logger.error(f"Cannot connect to VelesDB at {VELESDB_URL}")
        logger.error("Make sure VelesDB server is running: cargo run --release")
        return

    # Create a collection
    logger.info("Creating collection 'documents'...")
    try:
        client.create_collection("documents", dimension=4, metric="cosine")
        logger.info("Collection created!")
    except requests.HTTPError as e:
        if e.response.status_code == 400:
            logger.info("Collection already exists, continuing...")
        else:
            raise

    # Insert some vectors
    logger.info("Inserting vectors...")
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
    logger.info(f"Inserted {result['count']} points")

    # Search for similar vectors
    logger.info("Searching for vectors similar to [1.0, 0.0, 0.0, 0.0]...")
    query = [1.0, 0.0, 0.0, 0.0]
    results = client.search("documents", query, top_k=3)

    logger.info("Search results:")
    for i, result in enumerate(results, 1):
        logger.info(f"  {i}. ID: {result['id']}, Score: {result['score']:.4f}")
        if result.get('payload'):
            logger.info(f"     Title: {result['payload'].get('title')}")

    # List collections
    logger.info(f"Collections: {client.list_collections()}")


if __name__ == "__main__":
    main()
