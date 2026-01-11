# 🐺 VelesDB v1.1.0 Release - Major Feature Release

## 📋 Summary

This PR merges `develop` into `main` for the **v1.1.0 release** (January 11, 2026).

**4 EPICs completed** with **100% feature parity** across all components.

---

## 🚀 Features

### EPIC-CORE-001: Multi-Query Fusion (MQG)
- `FusionStrategy` enum: `Average`, `Maximum`, `RRF { k }`, `Weighted { avg, max, hit }`
- `Collection::multi_query_search()` in all components
- VelesQL `NEAR_FUSED($vectors, fusion='rrf', k=60)` syntax
- Compatible with LangChain MultiQueryRetriever

### EPIC-CORE-002: Metadata-Only Collections & LIKE/ILIKE
- `CollectionType::MetadataOnly` for vector-free collections
- `Condition::Like` / `Condition::ILike` SQL pattern matching
- VelesQL `WHERE title ILIKE '%pattern%'` support

### EPIC-CORE-003: SOTA 2026 Performance Optimizations
- **Trigram Index** with Roaring Bitmaps (22-128x faster LIKE)
- **LRU Cache** with O(1) operations
- **LockFree Cache** with DashMap L1
- **Bloom Filter** for fast negative lookups
- **Dictionary Encoder** for column compression

### EPIC-CORE-005: Full Coverage Parity
- All features available in: Core, Mobile, WASM, CLI, TS SDK, LangChain, LlamaIndex

---

## ⚡ Performance (Benchmarked January 11, 2026)

| Mode | Recall@10 | Latency P50 | vs v1.0 |
|------|-----------|-------------|---------|
| Fast (ef=64) | 92.2% | **36µs** | 🆕 |
| Balanced (ef=128) | 98.8% | **57µs** | 🚀 **-80%** |
| Accurate (ef=256) | 100% | **130µs** | 🚀 **-72%** |
| Perfect (ef=2048) | 100% | **200µs** | 🚀 **-92%** |

---

## 📊 Coverage Matrix (100%)

| Feature | Core | Mobile | WASM | CLI | TS SDK | LangChain | LlamaIndex |
|---------|:----:|:------:|:----:|:---:|:------:|:---------:|:----------:|
| multi_query_search | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| hybrid_search | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| batch_search | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| text_search | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| LIKE/ILIKE | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Hamming/Jaccard | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| metadata_only | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| get_by_id | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| FusionStrategy | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |

---

## 🧪 Tests

| Suite | Status |
|-------|--------|
| Rust (cargo test --all) | ✅ 35+ tests |
| TypeScript SDK | ✅ 61/61 tests |
| EPIC-CORE-003 TDD | ✅ 107 tests |
| Mobile | ✅ 30 tests |
| WASM | ✅ 35 tests |

---

## 🔒 Security

- `vite` upgraded from `^5.3.0` to `^7.3.1` (CVE fix in tauri-rag-app demo)

---

## 📚 Documentation

- [USAGE_EXAMPLES_V1.1.0.md](docs/USAGE_EXAMPLES_V1.1.0.md) - Complete usage guide
- [BENCHMARKS.md](docs/BENCHMARKS.md) - Updated performance metrics
- [CHANGELOG.md](CHANGELOG.md) - Full release notes

---

## 📝 Commits (38 total)

```
0c47bcf chore: set release date v1.1.0 to 2026-01-11
13c608d docs: add benchmark interpretation note
e3d84f5 fix(docs): correct benchmark metrics
df87608 test: fix TypeScript SDK test
f4d21be docs: clarify coverage matrix
e0e6df4 docs: add USAGE_EXAMPLES_V1.1.0.md
ce06052 security: upgrade vite (CVE fix)
67e2131 feat: complete 100% coverage matrix
5616835 Merge feature/EPIC-CORE-005-full-coverage
... and 29 more commits
```

---

## ✅ Checklist

- [x] All tests pass
- [x] Benchmarks show no regression (72-92% improvement)
- [x] Documentation updated
- [x] CHANGELOG updated with release date
- [x] Version 1.1.0 in all Cargo.toml/package.json
- [x] Security fixes applied

---

**Ready for merge to main** 🐺
