"""Tests for velesdb_common.security module."""

import os
import pytest
from velesdb_common.security import (
    SecurityError,
    validate_path,
    validate_dimension,
    validate_k,
    validate_text,
    validate_query,
    validate_metric,
    validate_storage_mode,
    validate_batch_size,
    validate_collection_name,
    validate_url,
    validate_weight,
    validate_timeout,
    validate_label,
    validate_node_id,
    MAX_QUERY_LENGTH,
    MAX_TEXT_LENGTH,
    MAX_BATCH_SIZE,
    MAX_K_VALUE,
    MAX_DIMENSION,
    MIN_DIMENSION,
    MAX_LABEL_LENGTH,
    MAX_NODE_ID,
    ALLOWED_METRICS,
    ALLOWED_STORAGE_MODES,
    DEFAULT_TIMEOUT_MS,
)


class TestValidatePath:
    """Tests for validate_path."""

    def test_valid_relative_path(self):
        result = validate_path("./data")
        assert os.path.isabs(result)

    def test_valid_absolute_path(self):
        result = validate_path(os.path.abspath("."))
        assert os.path.isabs(result)

    def test_empty_path_rejected(self):
        with pytest.raises(SecurityError, match="cannot be empty"):
            validate_path("")

    def test_null_bytes_rejected(self):
        with pytest.raises(SecurityError, match="null bytes"):
            validate_path("/safe/path\x00malicious")

    def test_parent_traversal_rejected(self):
        with pytest.raises(SecurityError, match="Suspicious"):
            validate_path("../../../etc/passwd")

    def test_unc_path_rejected(self):
        with pytest.raises(SecurityError, match="Suspicious"):
            validate_path("//network/share")

    def test_too_long_path_rejected(self):
        with pytest.raises(SecurityError, match="maximum length"):
            validate_path("a" * 5000)

    def test_sandbox_valid_subpath(self):
        """Path under base_directory is accepted."""
        base = os.path.abspath(".")
        sub = os.path.join(base, "data", "collections")
        result = validate_path(sub, base_directory=base)
        assert result.startswith(base)

    def test_sandbox_rejects_escape(self):
        """Path escaping sandbox via absolute path is rejected."""
        import tempfile
        base = os.path.join(tempfile.gettempdir(), "sandbox_test")
        outside = os.path.abspath(".")
        with pytest.raises(SecurityError, match="outside sandbox"):
            validate_path(outside, base_directory=base)

    def test_sandbox_base_itself_accepted(self):
        """The base directory path itself is accepted."""
        base = os.path.abspath(".")
        result = validate_path(base, base_directory=base)
        assert result == base

    def test_no_sandbox_by_default(self):
        """Without base_directory, any valid absolute path is accepted."""
        result = validate_path(".")
        assert os.path.isabs(result)


class TestValidateDimension:
    """Tests for validate_dimension."""

    def test_valid_dimension(self):
        assert validate_dimension(128) == 128

    def test_min_dimension(self):
        assert validate_dimension(MIN_DIMENSION) == MIN_DIMENSION

    def test_max_dimension(self):
        assert validate_dimension(MAX_DIMENSION) == MAX_DIMENSION

    def test_zero_rejected(self):
        with pytest.raises(SecurityError, match="at least"):
            validate_dimension(0)

    def test_negative_rejected(self):
        with pytest.raises(SecurityError, match="at least"):
            validate_dimension(-1)

    def test_too_large_rejected(self):
        with pytest.raises(SecurityError, match="maximum"):
            validate_dimension(MAX_DIMENSION + 1)

    def test_non_int_rejected(self):
        with pytest.raises(SecurityError, match="integer"):
            validate_dimension(128.0)  # type: ignore


class TestValidateK:
    """Tests for validate_k."""

    def test_valid_k(self):
        assert validate_k(10) == 10

    def test_k_one(self):
        assert validate_k(1) == 1

    def test_max_k(self):
        assert validate_k(MAX_K_VALUE) == MAX_K_VALUE

    def test_zero_rejected(self):
        with pytest.raises(SecurityError, match="at least 1"):
            validate_k(0)

    def test_too_large_rejected(self):
        with pytest.raises(SecurityError, match="maximum"):
            validate_k(MAX_K_VALUE + 1)

    def test_custom_param_name(self):
        with pytest.raises(SecurityError, match="similarity_top_k"):
            validate_k(0, param_name="similarity_top_k")

    def test_non_int_rejected(self):
        with pytest.raises(SecurityError, match="integer"):
            validate_k(10.5)  # type: ignore


class TestValidateText:
    """Tests for validate_text."""

    def test_valid_text(self):
        assert validate_text("hello world") == "hello world"

    def test_empty_text_ok(self):
        assert validate_text("") == ""

    def test_too_long_rejected(self):
        with pytest.raises(SecurityError, match="maximum length"):
            validate_text("a" * (MAX_TEXT_LENGTH + 1))

    def test_custom_max_length(self):
        with pytest.raises(SecurityError, match="maximum length"):
            validate_text("hello", max_length=3)

    def test_non_string_rejected(self):
        with pytest.raises(SecurityError, match="string"):
            validate_text(123)  # type: ignore


class TestValidateQuery:
    """Tests for validate_query."""

    def test_valid_select(self):
        assert validate_query("SELECT * FROM docs LIMIT 10") == "SELECT * FROM docs LIMIT 10"

    def test_valid_parameterized(self):
        q = "SELECT * FROM docs WHERE category = $cat"
        assert validate_query(q) == q

    def test_valid_match(self):
        q = "MATCH (a)-[:KNOWS]->(b) RETURN a.name"
        assert validate_query(q) == q

    def test_valid_near_vector(self):
        q = "SELECT * FROM docs WHERE embedding NEAR $v LIMIT 10"
        assert validate_query(q) == q

    def test_valid_union(self):
        q = "SELECT * FROM a UNION SELECT * FROM b"
        assert validate_query(q) == q

    def test_valid_similarity(self):
        q = "SELECT * FROM docs WHERE similarity(embedding, $v) > 0.8"
        assert validate_query(q) == q

    def test_null_bytes_rejected(self):
        with pytest.raises(SecurityError, match="null bytes"):
            validate_query("SELECT *\x00 DROP TABLE")

    def test_too_long_rejected(self):
        with pytest.raises(SecurityError, match="maximum length"):
            validate_query("S" * (MAX_QUERY_LENGTH + 1))

    def test_non_string_rejected(self):
        with pytest.raises(SecurityError, match="string"):
            validate_query(42)  # type: ignore

    def test_stacked_drop_rejected(self):
        with pytest.raises(SecurityError, match="Stacked query"):
            validate_query("SELECT * FROM docs; DROP TABLE docs")

    def test_stacked_delete_rejected(self):
        with pytest.raises(SecurityError, match="Stacked query"):
            validate_query("SELECT * FROM docs; DELETE FROM docs")

    def test_stacked_insert_rejected(self):
        with pytest.raises(SecurityError, match="Stacked query"):
            validate_query("SELECT 1; INSERT INTO evil VALUES(1)")

    def test_stacked_update_rejected(self):
        with pytest.raises(SecurityError, match="Stacked query"):
            validate_query("SELECT 1; UPDATE docs SET x=1")

    def test_line_comment_rejected(self):
        with pytest.raises(SecurityError, match="comment injection"):
            validate_query("SELECT * FROM docs -- WHERE admin=true")

    def test_block_comment_rejected(self):
        with pytest.raises(SecurityError, match="block comment"):
            validate_query("SELECT * FROM docs /* hidden */")

    def test_hex_escape_rejected(self):
        with pytest.raises(SecurityError, match="Hex escape"):
            validate_query("SELECT * FROM docs WHERE id = 0x414243")


class TestValidateMetric:
    """Tests for validate_metric."""

    def test_all_valid_metrics(self):
        for metric in ALLOWED_METRICS:
            assert validate_metric(metric) == metric

    def test_case_insensitive(self):
        assert validate_metric("COSINE") == "cosine"
        assert validate_metric("Euclidean") == "euclidean"

    def test_invalid_metric_rejected(self):
        with pytest.raises(SecurityError, match="Invalid metric"):
            validate_metric("manhattan")

    def test_non_string_rejected(self):
        with pytest.raises(SecurityError, match="string"):
            validate_metric(42)  # type: ignore


class TestValidateStorageMode:
    """Tests for validate_storage_mode."""

    def test_all_valid_modes(self):
        for mode in ALLOWED_STORAGE_MODES:
            assert validate_storage_mode(mode) == mode

    def test_case_insensitive(self):
        assert validate_storage_mode("SQ8") == "sq8"
        assert validate_storage_mode("BINARY") == "binary"

    def test_invalid_mode_rejected(self):
        with pytest.raises(SecurityError, match="Invalid storage mode"):
            validate_storage_mode("float16")

    def test_non_string_rejected(self):
        with pytest.raises(SecurityError, match="string"):
            validate_storage_mode(8)  # type: ignore


class TestValidateBatchSize:
    """Tests for validate_batch_size."""

    def test_valid_size(self):
        assert validate_batch_size(100) == 100

    def test_max_size(self):
        assert validate_batch_size(MAX_BATCH_SIZE) == MAX_BATCH_SIZE

    def test_too_large_rejected(self):
        with pytest.raises(SecurityError, match="maximum"):
            validate_batch_size(MAX_BATCH_SIZE + 1)


class TestValidateCollectionName:
    """Tests for validate_collection_name."""

    def test_valid_name(self):
        assert validate_collection_name("my_collection") == "my_collection"

    def test_alphanumeric_hyphen(self):
        assert validate_collection_name("docs-v2") == "docs-v2"

    def test_empty_rejected(self):
        with pytest.raises(SecurityError, match="cannot be empty"):
            validate_collection_name("")

    def test_special_chars_rejected(self):
        with pytest.raises(SecurityError, match="alphanumeric"):
            validate_collection_name("my collection!")

    def test_too_long_rejected(self):
        with pytest.raises(SecurityError, match="maximum length"):
            validate_collection_name("a" * 257)

    def test_non_string_rejected(self):
        with pytest.raises(SecurityError, match="string"):
            validate_collection_name(42)  # type: ignore


class TestValidateUrl:
    """Tests for validate_url."""

    def test_valid_http(self):
        assert validate_url("http://localhost:8080") == "http://localhost:8080"

    def test_valid_https(self):
        assert validate_url("https://api.velesdb.io") == "https://api.velesdb.io"

    def test_empty_rejected(self):
        with pytest.raises(SecurityError, match="cannot be empty"):
            validate_url("")

    def test_no_scheme_rejected(self):
        with pytest.raises(SecurityError, match="http://"):
            validate_url("localhost:8080")

    def test_ftp_rejected(self):
        with pytest.raises(SecurityError, match="http://"):
            validate_url("ftp://server/data")

    def test_newline_rejected(self):
        with pytest.raises(SecurityError, match="invalid characters"):
            validate_url("http://host\nHeader-Injection: evil")

    def test_non_string_rejected(self):
        with pytest.raises(SecurityError, match="string"):
            validate_url(42)  # type: ignore


class TestValidateWeight:
    """Tests for validate_weight."""

    def test_valid_weight(self):
        assert validate_weight(0.5) == 0.5

    def test_zero_ok(self):
        assert validate_weight(0.0) == 0.0

    def test_one_ok(self):
        assert validate_weight(1.0) == 1.0

    def test_int_one_ok(self):
        assert validate_weight(1) == 1.0

    def test_negative_rejected(self):
        with pytest.raises(SecurityError, match="between 0.0 and 1.0"):
            validate_weight(-0.1)

    def test_above_one_rejected(self):
        with pytest.raises(SecurityError, match="between 0.0 and 1.0"):
            validate_weight(1.1)

    def test_custom_name(self):
        with pytest.raises(SecurityError, match="vector_weight"):
            validate_weight(2.0, name="vector_weight")

    def test_non_number_rejected(self):
        with pytest.raises(SecurityError, match="number"):
            validate_weight("0.5")  # type: ignore


class TestValidateTimeout:
    """Tests for validate_timeout."""

    def test_valid_timeout(self):
        assert validate_timeout(5000) == 5000

    def test_min_timeout(self):
        assert validate_timeout(1) == 1

    def test_max_timeout(self):
        assert validate_timeout(DEFAULT_TIMEOUT_MS) == DEFAULT_TIMEOUT_MS

    def test_zero_rejected(self):
        with pytest.raises(SecurityError, match="at least 1ms"):
            validate_timeout(0)

    def test_too_large_rejected(self):
        with pytest.raises(SecurityError, match="maximum"):
            validate_timeout(DEFAULT_TIMEOUT_MS + 1)

    def test_non_int_rejected(self):
        with pytest.raises(SecurityError, match="integer"):
            validate_timeout(5.0)  # type: ignore


class TestValidateLabel:
    """Tests for validate_label."""

    def test_validate_label_valid(self):
        assert validate_label("PERSON") == "PERSON"
        assert validate_label("KNOWS") == "KNOWS"
        assert validate_label("works_at") == "works_at"
        assert validate_label("has-role") == "has-role"

    def test_validate_label_empty(self):
        with pytest.raises(SecurityError, match="cannot be empty"):
            validate_label("")

    def test_validate_label_injection(self):
        with pytest.raises(SecurityError, match="alphanumeric"):
            validate_label('"; DROP TABLE')

    def test_validate_label_special_chars(self):
        with pytest.raises(SecurityError, match="alphanumeric"):
            validate_label("KNOWS (well)")

    def test_validate_label_too_long(self):
        with pytest.raises(SecurityError, match="maximum length"):
            validate_label("A" * 200)

    def test_validate_label_null_bytes(self):
        with pytest.raises(SecurityError, match="null bytes"):
            validate_label("KNOWS\x00evil")

    def test_validate_label_non_string(self):
        with pytest.raises(SecurityError, match="string"):
            validate_label(42)  # type: ignore

    def test_validate_label_max_length_ok(self):
        label = "A" * MAX_LABEL_LENGTH
        assert validate_label(label) == label


class TestValidateNodeId:
    """Tests for validate_node_id."""

    def test_validate_node_id_valid(self):
        assert validate_node_id(0) == 0
        assert validate_node_id(1) == 1
        assert validate_node_id(42) == 42
        assert validate_node_id(2**62) == 2**62

    def test_validate_node_id_negative(self):
        with pytest.raises(SecurityError, match="non-negative"):
            validate_node_id(-1)

    def test_validate_node_id_too_large(self):
        with pytest.raises(SecurityError, match="maximum"):
            validate_node_id(2**64)

    def test_validate_node_id_non_int(self):
        with pytest.raises(SecurityError, match="integer"):
            validate_node_id("abc")  # type: ignore

    def test_validate_node_id_float_rejected(self):
        with pytest.raises(SecurityError, match="integer"):
            validate_node_id(1.5)  # type: ignore

    def test_validate_node_id_bool_rejected(self):
        with pytest.raises(SecurityError, match="integer"):
            validate_node_id(True)  # type: ignore

    def test_validate_node_id_max_ok(self):
        assert validate_node_id(MAX_NODE_ID) == MAX_NODE_ID

    def test_validate_node_id_over_max(self):
        with pytest.raises(SecurityError, match="maximum"):
            validate_node_id(MAX_NODE_ID + 1)
