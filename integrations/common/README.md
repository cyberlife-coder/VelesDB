# velesdb-common

Shared utilities for [VelesDB](https://github.com/cyberlife-coder/VelesDB) Python integrations.

> **This package is not a public API.** End users should install and import from
> [`langchain-velesdb`](https://pypi.org/project/langchain-velesdb/) or
> [`llama-index-vector-stores-velesdb`](https://pypi.org/project/llama-index-vector-stores-velesdb/)
> directly.

## What it provides

`velesdb-common` centralizes code shared by both integration packages:

- **Security validators** — input sanitization for collection names, dimensions, queries, URLs
- **ID generation** — deterministic hashing and sequential ID counters
- **Graph helpers** — REST payload builders and native graph bindings
- **Memory formatting** — procedural memory result formatting

## Public Exports

The following symbols form the stable internal API consumed by `langchain-velesdb`
and `llama-index-vector-stores-velesdb`. They are not intended for direct use by
end users.

| Export | Type | Description |
|--------|------|-------------|
| `build_fusion_strategy` | function | Converts a strategy name + params dict into the native `FusionStrategy` object |
| `validate_url` | function | Validates a server URL string; raises `ValueError` on malformed input |
| `validate_text` | function | Sanitizes free-text query strings against injection patterns |
| `validate_collection_name` | function | Ensures a collection name meets naming rules (alphanumeric + underscore, ≤ 64 chars) |
| `CollectionAdminMixin` | class | Mixin providing shared admin operations (`flush`, `get_collection_info`, `is_empty`) |
| `GraphOpsBase` | class | Base class for graph query helpers used by both integrations |

## License

MIT License (this integration). See [LICENSE](https://github.com/cyberlife-coder/VelesDB/blob/main/integrations/common/LICENSE) for details.

VelesDB Core itself is licensed under the [VelesDB Core License 1.0](https://github.com/cyberlife-coder/VelesDB/blob/main/LICENSE) (based on ELv2).
