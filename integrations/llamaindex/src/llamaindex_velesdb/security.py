"""Security utilities for VelesDB LlamaIndex integration.

Provides input validation, sanitization, and security constants.
"""

import os
import re
from typing import Any, Optional

# Security constants
MAX_QUERY_LENGTH = 10_000  # Max characters for VelesQL queries
MAX_TEXT_LENGTH = 1_000_000  # Max characters per document (1MB)
MAX_BATCH_SIZE = 10_000  # Max documents per batch operation
MAX_K_VALUE = 10_000  # Max top_k for search
MAX_DIMENSION = 65_536  # Max vector dimension (reasonable for any model)
MIN_DIMENSION = 1
MAX_PATH_LENGTH = 4096  # Max path length
ALLOWED_METRICS = {"cosine", "euclidean", "dot", "hamming", "jaccard"}
DEFAULT_TIMEOUT_MS = 30_000  # 30 seconds max timeout


class SecurityError(ValueError):
    """Raised when a security validation fails."""
    pass


def validate_path(path: str) -> str:
    """Validate and normalize a filesystem path.
    
    Prevents path traversal attacks and validates path safety.
    
    Args:
        path: The path to validate.
        
    Returns:
        Normalized absolute path.
        
    Raises:
        SecurityError: If path is invalid or potentially malicious.
    """
    if not path:
        raise SecurityError("Path cannot be empty")
    
    if len(path) > MAX_PATH_LENGTH:
        raise SecurityError(f"Path exceeds maximum length of {MAX_PATH_LENGTH}")
    
    # Normalize the path
    try:
        normalized = os.path.normpath(path)
        abs_path = os.path.abspath(normalized)
    except (ValueError, OSError) as e:
        raise SecurityError(f"Invalid path: {e}")
    
    # Check for null bytes (path injection)
    if "\x00" in path:
        raise SecurityError("Path contains null bytes")
    
    # Check for suspicious patterns
    suspicious_patterns = [
        r"\.\.[/\\]",  # Parent directory traversal
        r"^[/\\]{2}",  # UNC paths (network shares)
    ]
    
    for pattern in suspicious_patterns:
        if re.search(pattern, path):
            raise SecurityError(f"Suspicious path pattern detected")
    
    return abs_path


def validate_dimension(dimension: int) -> int:
    """Validate vector dimension.
    
    Args:
        dimension: Vector dimension to validate.
        
    Returns:
        Validated dimension.
        
    Raises:
        SecurityError: If dimension is out of valid range.
    """
    if not isinstance(dimension, int):
        raise SecurityError(f"Dimension must be an integer, got {type(dimension).__name__}")
    
    if dimension < MIN_DIMENSION:
        raise SecurityError(f"Dimension must be at least {MIN_DIMENSION}")
    
    if dimension > MAX_DIMENSION:
        raise SecurityError(f"Dimension exceeds maximum of {MAX_DIMENSION}")
    
    return dimension


def validate_k(k: int, param_name: str = "similarity_top_k") -> int:
    """Validate top-k parameter.
    
    Args:
        k: Number of results to return.
        param_name: Parameter name for error messages.
        
    Returns:
        Validated k value.
        
    Raises:
        SecurityError: If k is invalid.
    """
    if not isinstance(k, int):
        raise SecurityError(f"{param_name} must be an integer, got {type(k).__name__}")
    
    if k < 1:
        raise SecurityError(f"{param_name} must be at least 1")
    
    if k > MAX_K_VALUE:
        raise SecurityError(f"{param_name} exceeds maximum of {MAX_K_VALUE}")
    
    return k


def validate_text(text: str, max_length: int = MAX_TEXT_LENGTH) -> str:
    """Validate text content.
    
    Args:
        text: Text to validate.
        max_length: Maximum allowed length.
        
    Returns:
        Validated text.
        
    Raises:
        SecurityError: If text is invalid.
    """
    if not isinstance(text, str):
        raise SecurityError(f"Text must be a string, got {type(text).__name__}")
    
    if len(text) > max_length:
        raise SecurityError(f"Text exceeds maximum length of {max_length}")
    
    return text


def validate_query(query: str) -> str:
    """Validate VelesQL query string.
    
    Args:
        query: VelesQL query to validate.
        
    Returns:
        Validated query.
        
    Raises:
        SecurityError: If query is invalid or potentially dangerous.
    """
    if not isinstance(query, str):
        raise SecurityError(f"Query must be a string, got {type(query).__name__}")
    
    if len(query) > MAX_QUERY_LENGTH:
        raise SecurityError(f"Query exceeds maximum length of {MAX_QUERY_LENGTH}")
    
    # Null bytes could cause truncation issues
    if "\x00" in query:
        raise SecurityError("Query contains null bytes")
    
    return query


def validate_metric(metric: str) -> str:
    """Validate distance metric.
    
    Args:
        metric: Distance metric name.
        
    Returns:
        Validated metric (lowercase).
        
    Raises:
        SecurityError: If metric is not allowed.
    """
    if not isinstance(metric, str):
        raise SecurityError(f"Metric must be a string, got {type(metric).__name__}")
    
    metric_lower = metric.lower()
    if metric_lower not in ALLOWED_METRICS:
        raise SecurityError(
            f"Invalid metric '{metric}'. Allowed: {', '.join(sorted(ALLOWED_METRICS))}"
        )
    
    return metric_lower


def validate_batch_size(size: int) -> int:
    """Validate batch operation size.
    
    Args:
        size: Number of items in batch.
        
    Returns:
        Validated size.
        
    Raises:
        SecurityError: If size exceeds limit.
    """
    if size > MAX_BATCH_SIZE:
        raise SecurityError(
            f"Batch size {size} exceeds maximum of {MAX_BATCH_SIZE}. "
            f"Process in smaller batches."
        )
    
    return size


def validate_collection_name(name: str) -> str:
    """Validate collection name.
    
    Args:
        name: Collection name.
        
    Returns:
        Validated name.
        
    Raises:
        SecurityError: If name is invalid.
    """
    if not isinstance(name, str):
        raise SecurityError(f"Collection name must be a string, got {type(name).__name__}")
    
    if not name:
        raise SecurityError("Collection name cannot be empty")
    
    if len(name) > 256:
        raise SecurityError("Collection name exceeds maximum length of 256")
    
    # Only allow alphanumeric, underscore, hyphen
    if not re.match(r"^[a-zA-Z0-9_-]+$", name):
        raise SecurityError(
            "Collection name can only contain alphanumeric characters, underscores, and hyphens"
        )
    
    return name


def validate_url(url: str) -> str:
    """Validate server URL.
    
    Args:
        url: Server URL.
        
    Returns:
        Validated URL.
        
    Raises:
        SecurityError: If URL is invalid or potentially dangerous.
    """
    if not isinstance(url, str):
        raise SecurityError(f"URL must be a string, got {type(url).__name__}")
    
    if not url:
        raise SecurityError("URL cannot be empty")
    
    # Only allow http/https
    if not url.startswith(("http://", "https://")):
        raise SecurityError("URL must start with http:// or https://")
    
    # Check for common injection patterns
    if any(c in url for c in ["\n", "\r", "\x00"]):
        raise SecurityError("URL contains invalid characters")
    
    return url


def validate_weight(weight: float, name: str = "weight") -> float:
    """Validate a weight parameter (0.0 to 1.0).
    
    Args:
        weight: Weight value.
        name: Parameter name for error messages.
        
    Returns:
        Validated weight.
        
    Raises:
        SecurityError: If weight is out of range.
    """
    if not isinstance(weight, (int, float)):
        raise SecurityError(f"{name} must be a number, got {type(weight).__name__}")
    
    if weight < 0.0 or weight > 1.0:
        raise SecurityError(f"{name} must be between 0.0 and 1.0, got {weight}")
    
    return float(weight)


def validate_timeout(timeout_ms: int) -> int:
    """Validate timeout in milliseconds.
    
    Args:
        timeout_ms: Timeout value in milliseconds.
        
    Returns:
        Validated timeout.
        
    Raises:
        SecurityError: If timeout is invalid.
    """
    if not isinstance(timeout_ms, int):
        raise SecurityError(f"Timeout must be an integer, got {type(timeout_ms).__name__}")
    
    if timeout_ms < 1:
        raise SecurityError("Timeout must be at least 1ms")
    
    if timeout_ms > DEFAULT_TIMEOUT_MS:
        raise SecurityError(f"Timeout exceeds maximum of {DEFAULT_TIMEOUT_MS}ms (30s)")
    
    return timeout_ms
