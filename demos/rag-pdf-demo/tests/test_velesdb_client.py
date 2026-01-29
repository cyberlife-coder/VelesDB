"""Tests for VelesDB client module (TDD - tests first)."""

import pytest
from unittest.mock import AsyncMock, patch, MagicMock


class TestVelesDBClient:
    """Test suite for VelesDB REST client."""

    @pytest.fixture(autouse=True)
    def reset_client(self):
        """Reset the singleton client before each test."""
        from src.velesdb_client import VelesDBClient
        VelesDBClient._client = None
        VelesDBClient._lock = None
        VelesDBClient._lock_init = False
        yield
        VelesDBClient._client = None
        VelesDBClient._lock = None
        VelesDBClient._lock_init = False

    @pytest.mark.asyncio
    async def test_create_collection(self):
        """Test creating a collection in VelesDB."""
        from src.velesdb_client import VelesDBClient
        
        mock_response = MagicMock()
        mock_response.status_code = 200
        mock_response.json.return_value = {"name": "test_collection"}
        mock_response.raise_for_status = MagicMock()
        
        mock_http_client = AsyncMock()
        mock_http_client.post = AsyncMock(return_value=mock_response)
        mock_http_client.is_closed = False
        
        client = VelesDBClient(base_url="http://localhost:8080")
        
        with patch.object(client, '_get_client', return_value=mock_http_client):
            result = await client.create_collection("test_collection", 384)
            
        assert result["name"] == "test_collection"

    @pytest.mark.asyncio
    async def test_insert_vectors(self):
        """Test inserting vectors into VelesDB."""
        from src.velesdb_client import VelesDBClient
        
        mock_response = MagicMock()
        mock_response.status_code = 200
        mock_response.json.return_value = {"inserted": 2}
        mock_response.raise_for_status = MagicMock()
        
        mock_http_client = AsyncMock()
        mock_http_client.post = AsyncMock(return_value=mock_response)
        mock_http_client.is_closed = False
        
        client = VelesDBClient(base_url="http://localhost:8080")
        points = [
            {"id": 1, "vector": [0.1] * 384, "payload": {"text": "hello"}},
            {"id": 2, "vector": [0.2] * 384, "payload": {"text": "world"}}
        ]
        
        with patch.object(client, '_get_client', return_value=mock_http_client):
            result = await client.upsert_points("test_collection", points)
            
        assert result["inserted"] == 2

    @pytest.mark.asyncio
    async def test_search_vectors(self):
        """Test searching vectors in VelesDB."""
        from src.velesdb_client import VelesDBClient
        
        mock_response = MagicMock()
        mock_response.status_code = 200
        mock_response.json.return_value = {
            "results": [
                {"id": 1, "score": 0.95, "payload": {"text": "match"}},
                {"id": 2, "score": 0.80, "payload": {"text": "other"}}
            ]
        }
        mock_response.raise_for_status = MagicMock()
        
        mock_http_client = AsyncMock()
        mock_http_client.post = AsyncMock(return_value=mock_response)
        mock_http_client.is_closed = False
        
        client = VelesDBClient(base_url="http://localhost:8080")
        query_vector = [0.1] * 384
        
        with patch.object(client, '_get_client', return_value=mock_http_client):
            results = await client.search("test_collection", query_vector, top_k=5)
            
        assert len(results["results"]) == 2
        assert abs(results["results"][0]["score"] - 0.95) < 1e-6

    @pytest.mark.asyncio
    async def test_health_check(self):
        """Test VelesDB health check."""
        from src.velesdb_client import VelesDBClient
        
        mock_response = MagicMock()
        mock_response.status_code = 200
        mock_response.json.return_value = {"status": "healthy"}
        
        mock_http_client = AsyncMock()
        mock_http_client.get = AsyncMock(return_value=mock_response)
        mock_http_client.is_closed = False
        
        client = VelesDBClient(base_url="http://localhost:8080")
        
        with patch.object(client, '_get_client', return_value=mock_http_client):
            is_healthy = await client.health_check()
            
        assert is_healthy is True

    @pytest.mark.asyncio
    async def test_connection_error_handling(self):
        """Test handling of connection errors."""
        from src.velesdb_client import VelesDBClient, VelesDBConnectionError
        import httpx
        
        mock_http_client = AsyncMock()
        mock_http_client.get = AsyncMock(side_effect=httpx.ConnectError("Connection refused"))
        mock_http_client.is_closed = False
        
        client = VelesDBClient(base_url="http://localhost:8080")
        
        with patch.object(client, '_get_client', return_value=mock_http_client):
            with pytest.raises(VelesDBConnectionError):
                await client.health_check()
