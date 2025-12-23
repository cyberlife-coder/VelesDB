"""Tests for VelesDB client module (TDD - tests first)."""

import pytest
from unittest.mock import AsyncMock, patch, MagicMock


class TestVelesDBClient:
    """Test suite for VelesDB REST client."""

    @pytest.mark.asyncio
    async def test_create_collection(self):
        """Test creating a collection in VelesDB."""
        from src.velesdb_client import VelesDBClient
        
        with patch("httpx.AsyncClient") as mock_client:
            mock_response = MagicMock()
            mock_response.status_code = 200
            mock_response.json.return_value = {"name": "test_collection"}
            mock_client.return_value.__aenter__.return_value.post = AsyncMock(
                return_value=mock_response
            )
            
            client = VelesDBClient(base_url="http://localhost:8080")
            result = await client.create_collection("test_collection", 384)
            
            assert result["name"] == "test_collection"

    @pytest.mark.asyncio
    async def test_insert_vectors(self):
        """Test inserting vectors into VelesDB."""
        from src.velesdb_client import VelesDBClient
        
        with patch("httpx.AsyncClient") as mock_client:
            mock_response = MagicMock()
            mock_response.status_code = 200
            mock_response.json.return_value = {"inserted": 2}
            mock_client.return_value.__aenter__.return_value.post = AsyncMock(
                return_value=mock_response
            )
            
            client = VelesDBClient(base_url="http://localhost:8080")
            points = [
                {"id": 1, "vector": [0.1] * 384, "payload": {"text": "hello"}},
                {"id": 2, "vector": [0.2] * 384, "payload": {"text": "world"}}
            ]
            
            result = await client.upsert_points("test_collection", points)
            
            assert result["inserted"] == 2

    @pytest.mark.asyncio
    async def test_search_vectors(self):
        """Test searching vectors in VelesDB."""
        from src.velesdb_client import VelesDBClient
        
        with patch("httpx.AsyncClient") as mock_client:
            mock_response = MagicMock()
            mock_response.status_code = 200
            mock_response.json.return_value = {
                "results": [
                    {"id": 1, "score": 0.95, "payload": {"text": "match"}},
                    {"id": 2, "score": 0.80, "payload": {"text": "other"}}
                ]
            }
            mock_client.return_value.__aenter__.return_value.post = AsyncMock(
                return_value=mock_response
            )
            
            client = VelesDBClient(base_url="http://localhost:8080")
            query_vector = [0.1] * 384
            
            results = await client.search("test_collection", query_vector, top_k=5)
            
            assert len(results["results"]) == 2
            assert results["results"][0]["score"] == 0.95

    @pytest.mark.asyncio
    async def test_health_check(self):
        """Test VelesDB health check."""
        from src.velesdb_client import VelesDBClient
        
        with patch("httpx.AsyncClient") as mock_client:
            mock_response = MagicMock()
            mock_response.status_code = 200
            mock_response.json.return_value = {"status": "healthy"}
            mock_client.return_value.__aenter__.return_value.get = AsyncMock(
                return_value=mock_response
            )
            
            client = VelesDBClient(base_url="http://localhost:8080")
            is_healthy = await client.health_check()
            
            assert is_healthy is True

    @pytest.mark.asyncio
    async def test_connection_error_handling(self):
        """Test handling of connection errors."""
        from src.velesdb_client import VelesDBClient, VelesDBConnectionError
        import httpx
        
        with patch("httpx.AsyncClient") as mock_client:
            mock_client.return_value.__aenter__.return_value.get = AsyncMock(
                side_effect=httpx.ConnectError("Connection refused")
            )
            
            client = VelesDBClient(base_url="http://localhost:8080")
            
            with pytest.raises(VelesDBConnectionError):
                await client.health_check()
