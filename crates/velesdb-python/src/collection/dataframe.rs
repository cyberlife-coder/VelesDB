//! DataFrame convenience methods on `Collection`.
//!
//! Thin PyO3 wrappers that delegate to `velesdb.dataframe_converter` in Python.

use pyo3::prelude::*;

use crate::collection::Collection;
use crate::collection_helpers::core_err;

#[pymethods]
impl Collection {
    /// Convert search results to a DataFrame.
    ///
    /// Args:
    ///     results: List of search result dicts (id, score, payload).
    ///     backend: "pandas" or "polars" (default "pandas").
    ///
    /// Returns:
    ///     A pandas.DataFrame or polars.DataFrame.
    #[pyo3(signature = (results, *, backend="pandas"))]
    fn to_dataframe(&self, py: Python<'_>, results: PyObject, backend: &str) -> PyResult<PyObject> {
        let converter = py.import("velesdb.dataframe_converter")?;
        let df = converter.call_method1("to_dataframe", (results, backend))?;
        Ok(df.unbind())
    }

    /// Convert VelesQL query results to a DataFrame.
    ///
    /// Args:
    ///     results: List of result dicts from Collection.query().
    ///     backend: "pandas" or "polars" (default "pandas").
    ///
    /// Returns:
    ///     A pandas.DataFrame or polars.DataFrame.
    #[pyo3(signature = (results, *, backend="pandas"))]
    fn query_to_dataframe(
        &self,
        py: Python<'_>,
        results: PyObject,
        backend: &str,
    ) -> PyResult<PyObject> {
        let converter = py.import("velesdb.dataframe_converter")?;
        let df = converter.call_method1("query_to_dataframe", (results, backend))?;
        Ok(df.unbind())
    }

    /// Upsert points from a DataFrame.
    ///
    /// Args:
    ///     df: A pandas.DataFrame or polars.DataFrame with 'id', optional 'vector',
    ///         and payload columns.
    ///     backend: "pandas" or "polars" (default "pandas").
    ///
    /// Returns:
    ///     Number of upserted points.
    ///
    /// Raises:
    ///     ValueError: If required columns are missing or dimensions mismatch.
    #[pyo3(signature = (df, *, backend="pandas"))]
    fn upsert_from_dataframe(
        &self,
        py: Python<'_>,
        df: PyObject,
        backend: &str,
    ) -> PyResult<usize> {
        let _ = backend; // Backend auto-detected from DataFrame type
        let converter = py.import("velesdb.dataframe_converter")?;

        // Validate schema
        let config = self.inner.config();
        let metadata_only = config.metadata_only;
        let dimension = config.dimension;
        converter.call_method1("validate_upsert_dataframe", (&df, metadata_only, dimension))?;

        // Convert DataFrame to point dicts
        let points_list = converter.call_method1("dataframe_to_points", (&df,))?;
        let points: Vec<std::collections::HashMap<String, PyObject>> = points_list.extract()?;

        // Delegate to existing upsert path
        let parsed = crate::collection_helpers::parse_point_dicts(py, &points)?;
        let count = parsed.len();
        if metadata_only {
            self.inner.upsert_metadata(parsed).map_err(core_err)?;
        } else {
            self.inner.upsert(parsed).map_err(core_err)?;
        }
        Ok(count)
    }
}
