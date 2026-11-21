"""Tests for VelesQL Python bindings (EPIC-056 US-001)."""

import pytest


def test_velesql_parse_simple_select():
    """Test parsing a simple SELECT query."""
    from velesdb import VelesQL
    
    parsed = VelesQL.parse("SELECT * FROM documents LIMIT 10")
    
    assert parsed.is_valid()
    assert parsed.is_select()
    assert not parsed.is_match()
    assert parsed.collection_name == "documents"
    assert parsed.columns == ["*"]
    assert parsed.limit == 10
    assert parsed.offset is None


def test_velesql_parse_with_columns():
    """Test parsing SELECT with specific columns."""
    from velesdb import VelesQL
    
    parsed = VelesQL.parse("SELECT id, name, price FROM products")
    
    assert parsed.columns == ["id", "name", "price"]
    assert parsed.collection_name == "products"


def test_velesql_parse_with_where():
    """Test parsing SELECT with WHERE clause."""
    from velesdb import VelesQL
    
    parsed = VelesQL.parse("SELECT * FROM docs WHERE category = 'tech'")
    
    assert parsed.has_where_clause()
    assert not parsed.has_vector_search()


def test_velesql_parse_with_vector_search():
    """Test parsing query with vector search (NEAR)."""
    from velesdb import VelesQL
    
    parsed = VelesQL.parse("SELECT * FROM docs WHERE vector NEAR $query LIMIT 10")
    
    assert parsed.has_where_clause()
    assert parsed.has_vector_search()
    assert parsed.limit == 10


def test_velesql_parse_with_order_by():
    """Test parsing SELECT with ORDER BY."""
    from velesdb import VelesQL
    
    parsed = VelesQL.parse("SELECT * FROM docs ORDER BY created_at DESC")
    
    assert parsed.has_order_by()
    order_by = parsed.order_by
    assert len(order_by) == 1
    assert order_by[0][0] == "created_at"
    assert order_by[0][1] == "DESC"


def test_velesql_parse_with_multiple_order_by():
    """Test parsing SELECT with multiple ORDER BY columns."""
    from velesdb import VelesQL
    
    parsed = VelesQL.parse("SELECT * FROM docs ORDER BY priority DESC, name ASC")
    
    order_by = parsed.order_by
    assert len(order_by) == 2
    assert order_by[0] == ("priority", "DESC")
    assert order_by[1] == ("name", "ASC")


def test_velesql_parse_with_limit_offset():
    """Test parsing SELECT with LIMIT and OFFSET."""
    from velesdb import VelesQL
    
    parsed = VelesQL.parse("SELECT * FROM docs LIMIT 20 OFFSET 10")
    
    assert parsed.limit == 20
    assert parsed.offset == 10


def test_velesql_parse_with_distinct():
    """Test parsing SELECT DISTINCT."""
    from velesdb import VelesQL
    
    parsed = VelesQL.parse("SELECT DISTINCT category FROM products")
    
    assert parsed.has_distinct()
    assert parsed.columns == ["category"]


def test_velesql_parse_with_join():
    """Test parsing SELECT with JOIN."""
    from velesdb import VelesQL
    
    parsed = VelesQL.parse(
        "SELECT * FROM orders JOIN products ON orders.product_id = products.id"
    )
    
    assert parsed.has_joins()
    assert parsed.join_count == 1


def test_velesql_parse_with_group_by():
    """Test parsing SELECT with GROUP BY."""
    from velesdb import VelesQL
    
    parsed = VelesQL.parse(
        "SELECT category, COUNT(*) FROM products GROUP BY category"
    )
    
    assert parsed.has_group_by()
    assert "category" in parsed.group_by


def test_velesql_is_valid_static():
    """Test static is_valid method."""
    from velesdb import VelesQL
    
    assert VelesQL.is_valid("SELECT * FROM docs")
    assert VelesQL.is_valid("SELECT id FROM docs WHERE x = 1")
    assert not VelesQL.is_valid("SELEC * FROM docs")  # typo
    assert not VelesQL.is_valid("SELECT FROM docs")  # missing columns


def test_velesql_parse_syntax_error():
    """Test that syntax errors raise VelesQLSyntaxError."""
    from velesdb import VelesQL, VelesQLSyntaxError
    
    with pytest.raises(VelesQLSyntaxError):
        VelesQL.parse("SELEC * FROM docs")


def test_velesql_parsed_repr():
    """Test __repr__ of ParsedStatement."""
    from velesdb import VelesQL
    
    parsed = VelesQL.parse("SELECT * FROM documents")
    repr_str = repr(parsed)
    
    assert "ParsedStatement" in repr_str
    assert "SELECT" in repr_str
    assert "documents" in repr_str


def test_velesql_parsed_str():
    """Test __str__ of ParsedStatement."""
    from velesdb import VelesQL
    
    parsed = VelesQL.parse("SELECT id, name FROM users WHERE active = true LIMIT 10")
    str_output = str(parsed)
    
    assert "Type: SELECT" in str_output
    assert "Collection: users" in str_output
    assert "LIMIT: 10" in str_output


def test_velesql_parse_fusion():
    """Test parsing query with USING FUSION."""
    from velesdb import VelesQL
    
    parsed = VelesQL.parse(
        "SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 USING FUSION rrf"
    )
    
    assert parsed.has_fusion()
    assert parsed.has_vector_search()


def test_velesql_table_alias():
    """Test parsing query with table alias (for self-joins)."""
    from velesdb import VelesQL
    
    parsed = VelesQL.parse(
        "SELECT a.id FROM products a JOIN products b ON a.category = b.category"
    )
    
    assert parsed.collection_name == "products"
    assert parsed.table_alias == "a"
    assert parsed.table_aliases == ["a", "b"]


@pytest.mark.parametrize(
    ("sql", "table_alias", "table_aliases"),
    [
        pytest.param(
            "SELECT * FROM docs JOIN tags t ON t.id = docs.tag_id",
            None,
            ["t"],
            id="unaliased-from-aliased-join",
        ),
        pytest.param(
            "SELECT * FROM docs d JOIN tags t ON t.id = d.tag_id",
            "d",
            ["d", "t"],
            id="aliased-from-aliased-join",
        ),
        pytest.param(
            "SELECT * FROM docs AS d JOIN tags AS d USING (id)",
            "d",
            ["d", "d"],
            id="join-reuses-from-alias",
        ),
        pytest.param(
            "SELECT * FROM docs JOIN tags t ON t.id = docs.tag_id "
            "JOIN users u ON u.id = docs.owner_id",
            None,
            ["t", "u"],
            id="multiple-joins",
        ),
        pytest.param("SELECT * FROM docs AS d", "d", ["d"], id="single-aliased"),
        pytest.param("SELECT * FROM docs", None, [], id="single-unaliased"),
    ],
)
def test_table_alias_names_the_from_side_and_aliases_follow_declaration_order(
    sql, table_alias, table_aliases
):
    """The FROM alias and the ordered list of all alias declarations are exposed separately."""
    from velesdb import VelesQL

    parsed = VelesQL.parse(sql)

    assert parsed.table_alias == table_alias
    assert parsed.table_aliases == table_aliases


def test_a_parameter_error_raises_velesql_parameter_error(temp_db):
    """A ``$name`` with no binding, or bound to a value its position cannot
    take, raises ``VelesQLParameterError`` (VELES-038) — a ``ValueError``,
    so an existing ``except ValueError`` still catches it."""
    from velesdb import VelesQLParameterError

    assert issubclass(VelesQLParameterError, ValueError)
    col = temp_db.create_collection("unbound_param", dimension=4)
    col.upsert([{"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"n": 1}}])
    with pytest.raises(VelesQLParameterError, match=r"VELES-038.*\$v"):
        col.query("SELECT * FROM unbound_param WHERE vector NEAR $v LIMIT 5")
    with pytest.raises(VelesQLParameterError, match=r"VELES-038.*\$v"):
        col.query(
            "SELECT * FROM unbound_param WHERE vector NEAR $v LIMIT 5",
            params={"v": "not a vector"},
        )
    with pytest.raises(VelesQLParameterError, match=r"VELES-038.*\$n"):
        temp_db.execute_query("SELECT * FROM unbound_param WHERE n = $n LIMIT 5")
    with pytest.raises(ValueError, match=r"VELES-038.*\$n"):
        temp_db.execute_query("SELECT * FROM unbound_param WHERE n = $n LIMIT 5")
