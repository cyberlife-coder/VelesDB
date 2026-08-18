use super::*;

/// Tests that stringify a `PyErr` need a live interpreter (error
/// formatting goes through the Python C-API). Idempotent.
fn init_python() {
    Python::initialize();
}

// -- Section mapping: non-default values must propagate to core ------

#[test]
fn search_config_options_to_core_propagates_non_defaults() {
    let opts = SearchConfigOptions {
        default_mode: Some("accurate".to_string()),
        ef_search: Some(256),
        max_results: Some(42),
        query_timeout_ms: Some(5_000),
    };
    let core = opts.to_core().expect("valid section");
    assert!(matches!(core.default_mode, SearchMode::Accurate));
    assert_eq!(core.ef_search, Some(256));
    assert_eq!(core.max_results, 42);
    assert_eq!(core.query_timeout_ms, 5_000);
}

#[test]
fn search_config_options_unset_fields_keep_engine_defaults() {
    let core = SearchConfigOptions::default()
        .to_core()
        .expect("empty section is valid");
    let defaults = CoreSearchConfig::default();
    assert_eq!(core.max_results, defaults.max_results);
    assert_eq!(core.ef_search, defaults.ef_search);
    assert_eq!(core.query_timeout_ms, defaults.query_timeout_ms);
}

#[test]
fn search_config_options_rejects_unknown_mode() {
    let opts = SearchConfigOptions {
        default_mode: Some("warp".to_string()),
        ..SearchConfigOptions::default()
    };
    init_python();
    let err = opts.to_core().expect_err("unknown mode must fail");
    assert!(err.to_string().contains("default_mode"));
}

#[test]
fn hnsw_config_options_to_core_propagates_non_defaults() {
    let opts = HnswConfigOptions {
        m: Some(32),
        ef_construction: Some(400),
        max_layers: Some(8),
    };
    let core = opts.to_core();
    assert_eq!(core.m, Some(32));
    assert_eq!(core.ef_construction, Some(400));
    assert_eq!(core.max_layers, 8);
}

#[test]
fn storage_options_to_core_propagates_non_defaults() {
    let opts = StorageOptions {
        data_dir: Some("./custom".to_string()),
        storage_mode: Some("memory".to_string()),
        mmap_cache_mb: Some(256),
        vector_alignment: Some(32),
    };
    let core = opts.to_core();
    assert_eq!(core.data_dir, "./custom");
    assert_eq!(core.storage_mode, "memory");
    assert_eq!(core.mmap_cache_mb, 256);
    assert_eq!(core.vector_alignment, 32);
}

#[test]
fn quantization_options_to_core_propagates_pq_mode() {
    let opts = QuantizationOptions {
        mode: Some("pq".to_string()),
        pq_m: Some(8),
        pq_k: Some(128),
        pq_opq_enabled: Some(true),
        rerank_enabled: Some(false),
        rerank_multiplier: Some(3),
        ..QuantizationOptions::default()
    };
    let core = opts.to_core().expect("valid pq section");
    match core.mode {
        QuantizationType::PQ {
            m,
            k,
            opq_enabled,
            oversampling,
        } => {
            assert_eq!(m, 8);
            assert_eq!(k, 128);
            assert!(opq_enabled);
            assert_eq!(oversampling, Some(4), "unset oversampling → core default");
        }
        other => panic!("expected PQ mode, got {other:?}"),
    }
    assert!(!core.rerank_enabled);
    assert_eq!(core.rerank_multiplier, 3);
}

#[test]
fn quantization_options_pq_without_m_fails() {
    let opts = QuantizationOptions {
        mode: Some("pq".to_string()),
        ..QuantizationOptions::default()
    };
    init_python();
    let err = opts.to_core().expect_err("pq without pq_m must fail");
    assert!(err.to_string().contains("pq_m"));
}

#[test]
fn quantization_options_pq_fields_without_pq_mode_fail() {
    let opts = QuantizationOptions {
        mode: Some("sq8".to_string()),
        pq_m: Some(8),
        ..QuantizationOptions::default()
    };
    init_python();
    let err = opts
        .to_core()
        .expect_err("pq fields with mode='sq8' must fail, not be dropped");
    assert!(err.to_string().contains("pq_"));
}

// -- Whole-config assembly + validation ------------------------------

#[test]
fn veles_config_options_to_core_applies_every_section() {
    let opts = VelesConfigOptions {
        limits: Some(LimitsOptions {
            max_collections: Some(5),
            ..LimitsOptions::default()
        }),
        search: Some(SearchConfigOptions {
            max_results: Some(42),
            ..SearchConfigOptions::default()
        }),
        hnsw: Some(HnswConfigOptions {
            m: Some(32),
            ..HnswConfigOptions::default()
        }),
        storage: Some(StorageOptions {
            mmap_cache_mb: Some(256),
            ..StorageOptions::default()
        }),
        quantization: Some(QuantizationOptions {
            mode: Some("sq8".to_string()),
            ..QuantizationOptions::default()
        }),
    };
    let core = opts.to_core().expect("valid full config");
    assert_eq!(core.limits.max_collections, 5);
    assert_eq!(core.search.max_results, 42);
    assert_eq!(core.hnsw.m, Some(32));
    assert_eq!(core.storage.mmap_cache_mb, 256);
    assert!(matches!(core.quantization.mode, QuantizationType::SQ8));
}

#[test]
fn veles_config_options_to_core_validates_fail_fast() {
    let opts = VelesConfigOptions {
        search: Some(SearchConfigOptions {
            max_results: Some(0),
            ..SearchConfigOptions::default()
        }),
        ..VelesConfigOptions::default()
    };
    init_python();
    let err = opts.to_core().expect_err("max_results=0 must fail");
    assert!(err.to_string().contains("search.max_results"));
}

// -- TOML round-trip --------------------------------------------------

#[test]
fn from_core_round_trips_through_to_core() {
    let toml = "[limits]\nmax_collections = 7\n\n[search]\nmax_results = 42\n";
    let core = CoreVelesConfig::from_toml_engine_only(toml).expect("valid toml");
    let opts = VelesConfigOptions::from_core(&core).expect("mappable config");
    let back = opts.to_core().expect("round-trip stays valid");
    assert_eq!(back.limits.max_collections, 7);
    assert_eq!(back.search.max_results, 42);
    // Untouched engine defaults survive the round-trip:
    assert_eq!(back.storage.storage_mode, "mmap");
}
