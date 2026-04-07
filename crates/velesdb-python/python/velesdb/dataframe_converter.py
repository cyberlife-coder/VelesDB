"""DataFrame conversion utilities for VelesDB.

Converts between VelesDB result types (list[dict]) and Pandas/Polars
DataFrames. All pandas/polars imports are deferred to first use.
"""

from __future__ import annotations

import math
from typing import Any


def _import_pandas() -> Any:
    """Deferred pandas import with helpful error message."""
    try:
        import pandas  # noqa: F811
        return pandas
    except ImportError:
        raise ImportError(
            "pandas is required for DataFrame support. "
            "Install it with: pip install velesdb[pandas]"
        ) from None


def _import_polars() -> Any:
    """Deferred polars import with helpful error message."""
    try:
        import polars  # noqa: F811
        return polars
    except ImportError:
        raise ImportError(
            "polars is required for DataFrame support. "
            "Install it with: pip install velesdb[polars]"
        ) from None


def _get_backend(backend: str) -> Any:
    """Return the backend module for the given name."""
    if backend == "pandas":
        return _import_pandas()
    elif backend == "polars":
        return _import_polars()
    else:
        raise ValueError(
            f"Unsupported backend '{backend}'. Use 'pandas' or 'polars'"
        )


def to_dataframe(results: list[dict], backend: str = "pandas") -> Any:
    """Convert search results to a DataFrame.

    Each dict should have 'id', 'score', and optional payload keys.
    Missing payload fields are represented as None/NaN/null.

    Args:
        results: List of search result dicts.
        backend: "pandas" or "polars" (default "pandas").

    Returns:
        A pandas.DataFrame or polars.DataFrame.
    """
    lib = _get_backend(backend)

    if not results:
        if backend == "pandas":
            return lib.DataFrame(columns=["id", "score"])
        return lib.DataFrame(schema={"id": lib.Int64, "score": lib.Float64})

    rows = []
    for r in results:
        row: dict[str, Any] = {}
        payload = r.get("payload")
        if isinstance(payload, dict):
            row.update(payload)
        row["id"] = r.get("id")
        row["score"] = r.get("score")
        rows.append(row)

    return lib.DataFrame(rows)


def query_to_dataframe(rows: list[dict], backend: str = "pandas") -> Any:
    """Convert VelesQL query results to a DataFrame.

    One column per unique key across all dicts. Missing keys become null.

    Args:
        rows: List of result dicts from Collection.query().
        backend: "pandas" or "polars" (default "pandas").

    Returns:
        A pandas.DataFrame or polars.DataFrame.
    """
    lib = _get_backend(backend)

    if not rows:
        if backend == "pandas":
            return lib.DataFrame()
        return lib.DataFrame()

    return lib.DataFrame(rows)


def to_scroll_dataframe(batch: list[dict], backend: str = "pandas") -> Any:
    """Convert a scroll batch (list of point dicts) to a DataFrame.

    Each dict has 'id', 'vector', and 'payload' keys.

    Args:
        batch: List of point dicts from scroll.
        backend: "pandas" or "polars" (default "pandas").

    Returns:
        A pandas.DataFrame or polars.DataFrame.
    """
    lib = _get_backend(backend)

    if not batch:
        if backend == "pandas":
            return lib.DataFrame(columns=["id", "vector"])
        # Omit "payload" column: non-empty batches flatten dict payloads into
        # individual columns (no "payload" key), so the empty schema must match.
        return lib.DataFrame(
            schema={"id": lib.Int64, "vector": lib.List(lib.Float32)}
        )

    # Decide flattening strategy based on whether any point has a dict payload.
    # - If at least one point has a dict payload: flatten all dict payloads into
    #   top-level columns; non-dict payloads contribute no payload columns
    #   (missing keys become NaN/null naturally).
    # - If no point has a dict payload: store raw payload under a "payload" column.
    # This guarantees a consistent schema regardless of per-point payload type.
    has_dict_payload = any(isinstance(p.get("payload"), dict) for p in batch)

    rows = []
    for p in batch:
        row: dict[str, Any] = {}
        payload = p.get("payload")
        if has_dict_payload:
            if isinstance(payload, dict):
                row.update(payload)
            # Non-dict payloads in a mixed batch are silently skipped;
            # their columns will be NaN/null in the final DataFrame.
        else:
            row["payload"] = payload
        # Special columns are written last so payload keys cannot overwrite them.
        row["id"] = p.get("id")
        row["vector"] = list(p.get("vector", []))
        rows.append(row)

    return lib.DataFrame(rows)


def _df_to_records(df: Any) -> list[dict]:
    """Convert a DataFrame to a list of row dicts (backend-agnostic)."""
    if "polars" in type(df).__module__:
        return df.to_dicts()
    return df.to_dict(orient="records")


def _df_columns(df: Any) -> list[str]:
    """Return column names as a list (backend-agnostic)."""
    if "polars" in type(df).__module__:
        return df.columns
    return list(df.columns)


def _nan_to_none(value: Any) -> Any:
    """Convert float NaN to None for JSON-safe serialization."""
    if isinstance(value, float) and math.isnan(value):
        return None
    return value


def _row_to_point(row: dict) -> dict[str, Any]:
    """Convert a single row dict to a point dict for upsert.

    Note: only the first non-null vector is dimension-checked by
    validate_upsert_dataframe; subsequent mismatches surface as Rust errors.
    Pandas NaN values are converted to None for JSON compatibility.
    """
    point: dict[str, Any] = {"id": int(row["id"])}
    vec = row.get("vector")
    if vec is not None and not (isinstance(vec, float) and math.isnan(vec)):
        point["vector"] = list(vec) if not isinstance(vec, list) else vec
    payload = {k: _nan_to_none(v) for k, v in row.items() if k not in ("id", "vector")}
    if payload:
        point["payload"] = payload
    return point


def dataframe_to_points(df: Any) -> list[dict]:
    """Convert a DataFrame to a list of point dicts for upsert.

    Expected columns: 'id' (required), 'vector' (optional), plus payload columns.

    Args:
        df: A pandas.DataFrame or polars.DataFrame.

    Returns:
        List of point dicts with 'id', optional 'vector', and 'payload'.
    """
    return [_row_to_point(row) for row in _df_to_records(df)]


def _check_vector_dimension(df: Any, dimension: int) -> None:
    """Validate that the first non-null vector has the expected dimension.

    Note: only the first non-null vector is checked for performance. Later
    vectors with wrong dimensions surface as errors from the Rust core.
    """
    if "polars" in type(df).__module__:
        vectors = df["vector"].to_list()
    else:
        vectors = df["vector"].tolist()
    for vec in vectors:
        if vec is not None and not (isinstance(vec, float) and math.isnan(vec)):
            if len(vec) != dimension:
                raise ValueError(
                    f"Vector dimension mismatch: expected {dimension}, "
                    f"got {len(vec)}"
                )
            return


def validate_upsert_dataframe(
    df: Any, metadata_only: bool, dimension: int
) -> None:
    """Validate DataFrame schema before upsert.

    Args:
        df: A pandas.DataFrame or polars.DataFrame.
        metadata_only: Whether the target collection is metadata-only.
        dimension: Expected vector dimension.

    Raises:
        ValueError: If required columns are missing or dimensions mismatch.
    """
    columns = _df_columns(df)

    if "id" not in columns:
        raise ValueError("DataFrame must contain an 'id' column")

    if not metadata_only and "vector" not in columns:
        raise ValueError(
            "DataFrame must contain a 'vector' column for "
            "non-metadata-only collections"
        )

    if "vector" in columns and dimension > 0:
        _check_vector_dimension(df, dimension)
