//! `SearchOptions` builder for Python — VelesDB v1.15 additive API.
//!
//! Resolves issue #717: replaces the 6-kwargs `search()` signature with a
//! builder object that satisfies the project's `too_many_arguments` threshold
//! (≤5 args) without breaking the existing `search()` public API.
//!
//! # Migration path
//!
//! v1.14 (current)
//!   `collection.search(vector, top_k=10, filter={"k": "v"})`
//!
//! v1.15 (additive — no breaking change, two equivalent styles):
//!
//!   Keyword-constructor style:
//!   ```python
//!   opts = SearchOptions(vector=embedding, top_k=10, filter={"k": "v"})
//!   results = collection.search_request(opts)
//!   ```
//!
//!   Fluent builder style (chains return the same object):
//!   ```python
//!   results = collection.search_request(
//!       SearchOptions().with_vector(embedding).with_top_k(10)
//!   )
//!   ```
//!
//! v2.0 (breaking): `search()` kwargs path removed, `search_request()` is the
//! single canonical entry point.

use pyo3::prelude::*;

use crate::FusionStrategy;

/// Options for a vector search request.
///
/// Use as the argument to :py:meth:`Collection.search_request` (v1.15+).
/// All fields are optional except that at least one of ``vector`` or
/// ``sparse_vector`` must be set before calling ``search_request``.
///
/// Example:
///     >>> from velesdb import SearchOptions
///     >>> opts = SearchOptions(
///     ...     vector=my_embedding,
///     ...     top_k=20,
///     ...     filter={"category": "news"},
///     ... )
///     >>> results = collection.search_request(opts)
#[pyclass(module = "velesdb")]
pub struct SearchOptions {
    /// Dense query vector (list or numpy array). Optional when
    /// ``sparse_vector`` is provided.
    #[pyo3(get, set)]
    pub vector: Option<Py<PyAny>>,
    /// Sparse query as ``dict[int, float]`` or scipy sparse matrix.
    /// Optional when ``vector`` is provided.
    #[pyo3(get, set)]
    pub sparse_vector: Option<Py<PyAny>>,
    /// Number of results to return (default: 10).
    #[pyo3(get, set)]
    pub top_k: usize,
    /// Optional metadata filter dict.
    #[pyo3(get, set)]
    pub filter: Option<Py<PyAny>>,
    /// Named sparse index to query.  When ``None``, the default (unnamed)
    /// sparse index is used.
    #[pyo3(get, set)]
    pub sparse_index_name: Option<String>,
    /// When ``True`` the raw vector array is included in each result dict
    /// under the key ``"vector"``.  Disabled by default to keep response
    /// sizes small.
    #[pyo3(get, set)]
    pub include_vectors: bool,
    /// Optional fusion strategy applied when both ``vector`` and
    /// ``sparse_vector`` are provided (typed hybrid dense+sparse search).
    /// When ``None`` the default Reciprocal Rank Fusion (RRF, k=60) is used,
    /// preserving the historical behavior.  Has no effect on dense-only or
    /// sparse-only searches.
    #[pyo3(get, set)]
    pub fusion: Option<FusionStrategy>,
    /// Optional principal (caller identity) forwarded to the control-plane read
    /// gate. When ``None`` (default) and no observer is registered the gate is a
    /// zero-overhead allow, preserving pre-governance behavior.
    #[pyo3(get, set)]
    pub principal: Option<String>,
    /// Optional tenant hint forwarded to the control-plane read gate alongside
    /// ``principal`` for multi-tenant scoping. ``None`` by default.
    #[pyo3(get, set)]
    pub tenant: Option<String>,
}

#[pymethods]
impl SearchOptions {
    /// Create a new ``SearchOptions``.
    ///
    /// All arguments are keyword-only and optional at construction time;
    /// provide them here or assign to the corresponding attributes before
    /// passing to :py:meth:`Collection.search_request`.
    ///
    /// Args:
    ///     vector: Dense query vector.
    ///     sparse_vector: Sparse query (dict[int, float] or scipy sparse).
    ///     top_k: Max results to return (default: 10).
    ///     filter: Metadata pre-filter dict.
    ///     sparse_index_name: Named sparse index to query.
    ///     include_vectors: Include raw vectors in results (default: False).
    ///     fusion: Optional :py:class:`FusionStrategy` for hybrid dense+sparse
    ///         fusion (default: RRF k=60).
    ///     principal: Optional caller identity forwarded to the read gate.
    ///     tenant: Optional tenant hint forwarded to the read gate.
    #[new]
    #[allow(clippy::too_many_arguments)] // additive governance kwargs on an existing builder
    #[pyo3(signature = (
        vector = None,
        *,
        sparse_vector = None,
        top_k = 10,
        filter = None,
        sparse_index_name = None,
        include_vectors = false,
        fusion = None,
        principal = None,
        tenant = None,
    ))]
    pub fn new(
        vector: Option<Py<PyAny>>,
        sparse_vector: Option<Py<PyAny>>,
        top_k: usize,
        filter: Option<Py<PyAny>>,
        sparse_index_name: Option<String>,
        include_vectors: bool,
        fusion: Option<FusionStrategy>,
        principal: Option<String>,
        tenant: Option<String>,
    ) -> Self {
        Self {
            vector,
            sparse_vector,
            top_k,
            filter,
            sparse_index_name,
            include_vectors,
            fusion,
            principal,
            tenant,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "SearchOptions(top_k={}, include_vectors={}, sparse_index_name={:?})",
            self.top_k, self.include_vectors, self.sparse_index_name,
        )
    }
}

/// Fluent builder methods — each mutates `self` in place and returns the same
/// Python object so calls can be chained:
///
/// ```python
/// opts = SearchOptions().with_vector(emb).with_top_k(20).with_filter({"lang": "en"})
/// ```
///
/// The `Py<Self>` receiver pattern is the idiomatic PyO3 way to implement
/// builder chains: the Python object is borrowed mutably for the field write,
/// then returned unchanged so the caller owns the same reference.
#[pymethods]
impl SearchOptions {
    /// Sets the dense query vector and returns `self`.
    pub fn with_vector(slf: Py<Self>, py: Python<'_>, vector: Option<Py<PyAny>>) -> Py<Self> {
        slf.bind(py).borrow_mut().vector = vector;
        slf
    }

    /// Sets the sparse query vector and returns `self`.
    pub fn with_sparse_vector(
        slf: Py<Self>,
        py: Python<'_>,
        sparse_vector: Option<Py<PyAny>>,
    ) -> Py<Self> {
        slf.bind(py).borrow_mut().sparse_vector = sparse_vector;
        slf
    }

    /// Sets the number of results to return and returns `self`.
    pub fn with_top_k(slf: Py<Self>, py: Python<'_>, top_k: usize) -> Py<Self> {
        slf.bind(py).borrow_mut().top_k = top_k;
        slf
    }

    /// Sets the metadata filter and returns `self`.
    pub fn with_filter(slf: Py<Self>, py: Python<'_>, filter: Option<Py<PyAny>>) -> Py<Self> {
        slf.bind(py).borrow_mut().filter = filter;
        slf
    }

    /// Sets the named sparse index to query and returns `self`.
    pub fn with_sparse_index_name(slf: Py<Self>, py: Python<'_>, name: Option<String>) -> Py<Self> {
        slf.bind(py).borrow_mut().sparse_index_name = name;
        slf
    }

    /// Sets whether raw vectors are included in results and returns `self`.
    pub fn with_include_vectors(slf: Py<Self>, py: Python<'_>, include: bool) -> Py<Self> {
        slf.bind(py).borrow_mut().include_vectors = include;
        slf
    }

    /// Sets the hybrid dense+sparse fusion strategy and returns `self`.
    ///
    /// Pass ``None`` to fall back to the default RRF (k=60).
    pub fn with_fusion(slf: Py<Self>, py: Python<'_>, fusion: Option<FusionStrategy>) -> Py<Self> {
        slf.bind(py).borrow_mut().fusion = fusion;
        slf
    }

    /// Sets the read-gate principal (caller identity) and returns `self`.
    pub fn with_principal(slf: Py<Self>, py: Python<'_>, principal: Option<String>) -> Py<Self> {
        slf.bind(py).borrow_mut().principal = principal;
        slf
    }

    /// Sets the read-gate tenant hint and returns `self`.
    pub fn with_tenant(slf: Py<Self>, py: Python<'_>, tenant: Option<String>) -> Py<Self> {
        slf.bind(py).borrow_mut().tenant = tenant;
        slf
    }
}
