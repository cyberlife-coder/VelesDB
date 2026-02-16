"""
SDK Contract Tests for LangChain Integration

This module verifies that every method called on self._collection in the LangChain
integration actually exists on the real velesdb.Collection class. This prevents
phantom method regressions where integration code calls methods that don't exist
on the SDK.
"""

import ast
import re
from pathlib import Path
from typing import List, Set

import pytest


# Try to import velesdb for runtime verification
try:
    import velesdb
    HAS_VELESDB = True
except ImportError:
    HAS_VELESDB = False

MIN_VELESDB_VERSION = (1, 4, 0)


# List of all expected collection methods that should exist on velesdb.Collection
# This list should be kept in sync with the actual SDK methods
EXPECTED_COLLECTION_METHODS = [
    "upsert",
    "search", 
    "get", 
    "delete", 
    "flush", 
    "is_empty",
    "text_search", 
    "hybrid_search", 
    "batch_search", 
    "search_with_filter",
    "query", 
    "multi_query_search", 
    "info", 
    "is_metadata_only",
    "create_property_index", 
    "list_indexes", 
    "drop_index",
    "add_edge", 
    "get_edges", 
    "get_edges_by_label",
    "traverse", 
    "get_node_degree",
    "explain",
    "match_query",
    "upsert_bulk",
]


class CollectionMethodExtractor(ast.NodeVisitor):
    """AST visitor to extract all self._collection.xxx() method calls from source code."""
    
    def __init__(self):
        self.method_calls = set()
    
    def visit_Call(self, node: ast.Call) -> None:
        """Visit call nodes and check for self._collection.xxx() patterns."""
        if isinstance(node.func, ast.Attribute):
            # Check for self._collection.xxx() pattern
            if (isinstance(node.func.value, ast.Attribute) and
                node.func.value.attr == "_collection" and
                isinstance(node.func.value.value, ast.Name) and
                node.func.value.value.id == "self"):
                
                method_name = node.func.attr
                self.method_calls.add(method_name)
        
        # Continue visiting child nodes
        self.generic_visit(node)


def extract_collection_methods_from_source(file_path: str) -> Set[str]:
    """Extract all self._collection.xxx() method calls from a Python source file."""
    with open(file_path, 'r', encoding='utf-8') as f:
        source = f.read()
    
    tree = ast.parse(source)
    extractor = CollectionMethodExtractor()
    extractor.visit(tree)
    
    return extractor.method_calls


def get_velesdb_collection_methods() -> Set[str]:
    """Get all methods available on velesdb.Collection class."""
    if not HAS_VELESDB:
        return set()

    # PyO3 methods are exposed as method descriptors on builtins types.
    # `inspect.isfunction/ismethod` returns False, so use callable+dir().
    methods = set()
    for name in dir(velesdb.Collection):
        if name.startswith("_"):
            continue
        member = getattr(velesdb.Collection, name, None)
        if callable(member):
            methods.add(name)

    return methods


def parse_version_tuple(version_str: str) -> tuple[int, int, int]:
    """Parse semantic-like version string into (major, minor, patch)."""
    match = re.match(r"^(\d+)\.(\d+)\.(\d+)", version_str)
    if not match:
        return (0, 0, 0)
    return tuple(int(part) for part in match.groups())


def assert_supported_velesdb_runtime() -> None:
    """Fail early with actionable message when local velesdb runtime is stale."""
    if not HAS_VELESDB:
        return

    version_str = getattr(velesdb, "__version__", "0.0.0")
    version_tuple = parse_version_tuple(version_str)
    assert version_tuple >= MIN_VELESDB_VERSION, (
        f"Loaded velesdb runtime version {version_str} is too old for this contract test. "
        "Expected at least 1.4.0. Rebuild local bindings first: "
        "cd crates/velesdb-python && maturin develop --release"
    )


@pytest.mark.skipif(not HAS_VELESDB, reason="velesdb SDK not installed")
class TestSDKContractRuntime:
    """Runtime verification tests that require velesdb SDK to be installed."""

    def test_runtime_version_is_supported(self):
        """Ensure local runtime is recent enough for strict contract checks."""
        assert_supported_velesdb_runtime()

    def test_runtime_introspection_detects_methods(self):
        """Guard against false negatives from unsupported introspection strategy."""
        assert_supported_velesdb_runtime()
        sdk_methods = get_velesdb_collection_methods()
        assert len(sdk_methods) > 0, (
            "No callable methods detected on velesdb.Collection. "
            "If bindings were rebuilt, check runtime import path and rebuild: "
            "cd crates/velesdb-python && maturin develop --release"
        )
    
    def test_all_expected_methods_exist_on_sdk(self):
        """Verify that all expected collection methods exist on velesdb.Collection."""
        assert_supported_velesdb_runtime()
        sdk_methods = get_velesdb_collection_methods()
        
        missing_methods = []
        for expected_method in EXPECTED_COLLECTION_METHODS:
            if expected_method not in sdk_methods:
                missing_methods.append(expected_method)
        
        assert len(missing_methods) == 0, (
            f"The following expected methods are missing from velesdb.Collection: {missing_methods}"
        )
    
    def test_vectorstore_methods_exist_on_sdk(self):
        """Verify that methods used in vectorstore.py exist on velesdb.Collection."""
        assert_supported_velesdb_runtime()
        vectorstore_path = Path(__file__).parent.parent / "src" / "langchain_velesdb" / "vectorstore.py"
        
        if not vectorstore_path.exists():
            pytest.skip(f"Vectorstore file not found: {vectorstore_path}")
        
        used_methods = extract_collection_methods_from_source(vectorstore_path)
        sdk_methods = get_velesdb_collection_methods()
        
        missing_methods = used_methods - sdk_methods
        assert len(missing_methods) == 0, (
            f"Vectorstore uses methods that don't exist on velesdb.Collection: {missing_methods}"
        )


class TestSDKContractSource:
    """Source code analysis tests that don't require velesdb SDK."""
    
    def test_vectorstore_methods_in_expected_list(self):
        """Verify that all methods used in vectorstore.py are in the expected methods list."""
        vectorstore_path = Path(__file__).parent.parent / "src" / "langchain_velesdb" / "vectorstore.py"
        
        if not vectorstore_path.exists():
            pytest.skip(f"Vectorstore file not found: {vectorstore_path}")
        
        used_methods = extract_collection_methods_from_source(vectorstore_path)
        expected_methods = set(EXPECTED_COLLECTION_METHODS)
        
        unexpected_methods = used_methods - expected_methods
        assert len(unexpected_methods) == 0, (
            f"Vectorstore uses methods not in expected list: {unexpected_methods}. "
            f"Either update EXPECTED_COLLECTION_METHODS or fix the integration code."
        )
    
    def test_no_phantom_methods_in_vectorstore(self):
        """Verify that vectorstore.py doesn't call any phantom methods on _collection."""
        vectorstore_path = Path(__file__).parent.parent / "src" / "langchain_velesdb" / "vectorstore.py"
        
        if not vectorstore_path.exists():
            pytest.skip(f"Vectorstore file not found: {vectorstore_path}")
        
        # Read the source file
        with open(vectorstore_path, 'r', encoding='utf-8') as f:
            source = f.read()
        
        # Use regex to find all self._collection.xxx() patterns
        pattern = r'self\._collection\.(\w+)\s*\('
        matches = re.findall(pattern, source)
        
        # All matches should be in our expected methods list
        unexpected_methods = set(matches) - set(EXPECTED_COLLECTION_METHODS)
        
        assert len(unexpected_methods) == 0, (
            f"Found phantom method calls in vectorstore.py: {unexpected_methods}. "
            f"These methods don't exist in the expected SDK contract."
        )


if __name__ == "__main__":
    # Run the source-based tests (don't require velesdb)
    pytest.main([__file__, "-v", "-k", "Source"])
