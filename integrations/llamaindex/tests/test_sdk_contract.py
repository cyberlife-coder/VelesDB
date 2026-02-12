"""
SDK Contract Tests for LlamaIndex Integration

This module verifies that every method called on self._collection in the LlamaIndex
integration actually exists on the real velesdb.Collection class. This prevents
phantom method regressions where integration code calls methods that don't exist
on the SDK.
"""

import ast
import inspect
import re
from pathlib import Path
from typing import Set

import pytest


# Try to import velesdb for runtime verification
try:
    import velesdb
    HAS_VELESDB = True
except ImportError:
    HAS_VELESDB = False


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
    
    methods = set()
    for name, member in inspect.getmembers(velesdb.Collection):
        if inspect.isfunction(member) or inspect.ismethod(member):
            methods.add(name)
    
    return methods


@pytest.mark.skipif(not HAS_VELESDB, reason="velesdb SDK not installed")
class TestSDKContractRuntime:
    """Runtime verification tests that require velesdb SDK to be installed."""
    
    def test_all_expected_methods_exist_on_sdk(self):
        """Verify that all expected collection methods exist on velesdb.Collection."""
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
        vectorstore_path = Path(__file__).parent.parent / "src" / "llamaindex_velesdb" / "vectorstore.py"
        
        if not vectorstore_path.exists():
            pytest.skip(f"Vectorstore file not found: {vectorstore_path}")
        
        used_methods = extract_collection_methods_from_source(vectorstore_path)
        sdk_methods = get_velesdb_collection_methods()
        
        missing_methods = used_methods - sdk_methods
        assert len(missing_methods) == 0, (
            f"Vectorstore uses methods that don't exist on velesdb.Collection: {missing_methods}"
        )
    
    def test_graph_loader_methods_exist_on_sdk(self):
        """Verify that methods used in graph_loader.py exist on velesdb.Collection."""
        graph_loader_path = Path(__file__).parent.parent / "src" / "llamaindex_velesdb" / "graph_loader.py"
        
        if not graph_loader_path.exists():
            pytest.skip(f"Graph loader file not found: {graph_loader_path}")
        
        used_methods = extract_collection_methods_from_source(graph_loader_path)
        sdk_methods = get_velesdb_collection_methods()
        
        missing_methods = used_methods - sdk_methods
        assert len(missing_methods) == 0, (
            f"Graph loader uses methods that don't exist on velesdb.Collection: {missing_methods}"
        )


class TestSDKContractSource:
    """Source code analysis tests that don't require velesdb SDK."""
    
    def test_vectorstore_methods_in_expected_list(self):
        """Verify that all methods used in vectorstore.py are in the expected methods list."""
        vectorstore_path = Path(__file__).parent.parent / "src" / "llamaindex_velesdb" / "vectorstore.py"
        
        if not vectorstore_path.exists():
            pytest.skip(f"Vectorstore file not found: {vectorstore_path}")
        
        used_methods = extract_collection_methods_from_source(vectorstore_path)
        expected_methods = set(EXPECTED_COLLECTION_METHODS)
        
        unexpected_methods = used_methods - expected_methods
        assert len(unexpected_methods) == 0, (
            f"Vectorstore uses methods not in expected list: {unexpected_methods}. "
            f"Either update EXPECTED_COLLECTION_METHODS or fix the integration code."
        )
    
    def test_graph_loader_methods_in_expected_list(self):
        """Verify that all methods used in graph_loader.py are in the expected methods list."""
        graph_loader_path = Path(__file__).parent.parent / "src" / "llamaindex_velesdb" / "graph_loader.py"
        
        if not graph_loader_path.exists():
            pytest.skip(f"Graph loader file not found: {graph_loader_path}")
        
        used_methods = extract_collection_methods_from_source(graph_loader_path)
        expected_methods = set(EXPECTED_COLLECTION_METHODS)
        
        unexpected_methods = used_methods - expected_methods
        assert len(unexpected_methods) == 0, (
            f"Graph loader uses methods not in expected list: {unexpected_methods}. "
            f"Either update EXPECTED_COLLECTION_METHODS or fix the integration code."
        )
    
    def test_no_phantom_methods_in_vectorstore(self):
        """Verify that vectorstore.py doesn't call any phantom methods on _collection."""
        vectorstore_path = Path(__file__).parent.parent / "src" / "llamaindex_velesdb" / "vectorstore.py"
        
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
    
    def test_no_phantom_methods_in_graph_loader(self):
        """Verify that graph_loader.py doesn't call any phantom methods on _collection."""
        graph_loader_path = Path(__file__).parent.parent / "src" / "llamaindex_velesdb" / "graph_loader.py"
        
        if not graph_loader_path.exists():
            pytest.skip(f"Graph loader file not found: {graph_loader_path}")
        
        # Read the source file
        with open(graph_loader_path, 'r', encoding='utf-8') as f:
            source = f.read()
        
        # Use regex to find all self._collection.xxx() patterns
        pattern = r'self\._collection\.(\w+)\s*\('
        matches = re.findall(pattern, source)
        
        # All matches should be in our expected methods list
        unexpected_methods = set(matches) - set(EXPECTED_COLLECTION_METHODS)
        
        assert len(unexpected_methods) == 0, (
            f"Found phantom method calls in graph_loader.py: {unexpected_methods}. "
            f"These methods don't exist in the expected SDK contract."
        )


if __name__ == "__main__":
    # Run the source-based tests (don't require velesdb)
    pytest.main([__file__, "-v", "-k", "Source"])