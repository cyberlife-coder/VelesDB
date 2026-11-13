"""Embedder choice for the install wizard, checked against the existing store.

A store is readable only by the embedding model that filled it:
- a different **dimension** is refused by velesdb-core when the collection
  opens (VELES-004);
- a different **model of the same dimension** opens and then recalls noise,
  which is why velesdb-memory records `embedding-provenance.json`
  (`crates/velesdb-memory/src/embedding_provenance.rs`) and refuses a
  mismatch on a store that holds facts.

So the wizard never proposes a model blind: it reads the store, classifies
every candidate, and offers `velesdb-memory migrate-embeddings`
(docs/guides/MIGRATE_EMBEDDINGS.md) when the user picks one the store cannot
serve — switching embedders never costs the memories.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

#: The built-in offline embedder: deterministic, not semantic, 384-dim.
HASH_DIMENSION = 384
COLLECTIONS = ("_semantic_memory", "_episodic_memory", "_procedural_memory")
PROVENANCE_FILE = "embedding-provenance.json"
LINUX_INSTALL_SCRIPT = "https://ollama.com/install.sh"
OLLAMA_URL = "http://localhost:11434"

# compatibility verdicts
FRESH = "fresh"            # no store yet: it will be created at this model's dimension
MATCH = "match"            # the store's recorded model, or an empty store of the same dimension
UNVERIFIED = "unverified"  # legacy store without a provenance record: only the dimension can be checked
MIGRATE = "migrate"        # needs `velesdb-memory migrate-embeddings` (or another store)


@dataclass(frozen=True)
class Model:
    name: str
    dimension: int
    size: str
    note: str


#: The models the wizard proposes. Dimensions are the models' output sizes;
#: the wizard still probes the real one after a pull before trusting it.
CATALOG: tuple[Model, ...] = (
    Model("all-minilm", 384, "~46 MB", "light and fast"),
    Model("nomic-embed-text", 768, "~274 MB", "better recall, long context"),
    Model("mxbai-embed-large", 1024, "~670 MB", "high quality, English"),
    Model("bge-m3", 1024, "~1.2 GB", "high quality, multilingual"),
)


@dataclass(frozen=True)
class Embedder:
    backend: str  # "hash" | "ollama"
    model: str
    dimension: int


HASH = Embedder("hash", "hash", HASH_DIMENSION)


@dataclass(frozen=True)
class StoreProfile:
    exists: bool
    dimension: "int | None" = None
    model: "str | None" = None
    has_facts: bool = False


@dataclass(frozen=True)
class Candidate:
    name: str
    dimension: int
    size: str
    note: str
    installed: bool
    verdict: str


def _read_json(path: Path) -> "dict | None":
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    return data if isinstance(data, dict) else None


def _collection_dimension(configs: list[dict]) -> "int | None":
    dims = [c["dimension"] for c in configs if isinstance(c.get("dimension"), int)]
    return dims[0] if dims else None


def _holds_facts(configs: list[dict]) -> bool:
    return any(int(c.get("point_count") or 0) > 0 for c in configs)


def read_store(store: Path) -> StoreProfile:
    configs = [c for c in (_read_json(store / name / "config.json") for name in COLLECTIONS) if c]
    if not configs:
        return StoreProfile(exists=False)
    provenance = _read_json(store / PROVENANCE_FILE) or {}
    return StoreProfile(
        exists=True,
        dimension=_collection_dimension(configs) or provenance.get("dimension"),
        model=provenance.get("model"),
        has_facts=_holds_facts(configs),
    )


def compatibility(store: StoreProfile, model: str, dimension: int) -> str:
    if not store.exists:
        return FRESH
    if store.dimension != dimension:
        return MIGRATE
    if store.model is None:
        return UNVERIFIED
    if store.model == model or not store.has_facts:
        return MATCH
    return MIGRATE


def _bare(name: str) -> str:
    return name.split(":", 1)[0]


def rank_models(store: StoreProfile, local_models: "list[str]") -> list[Candidate]:
    """Catalog candidates, usable ones first, then installed before not."""
    installed = {_bare(m) for m in local_models}
    order = {MATCH: 0, FRESH: 0, UNVERIFIED: 1, MIGRATE: 2}
    candidates = [Candidate(m.name, m.dimension, m.size, m.note, m.name in installed,
                            compatibility(store, m.name, m.dimension)) for m in CATALOG]
    return sorted(candidates, key=lambda c: (order[c.verdict], not c.installed))


def local_models(tags: dict) -> list[str]:
    """Model names from Ollama's GET /api/tags response."""
    return [str(m.get("name")) for m in tags.get("models", []) if isinstance(m, dict) and m.get("name")]


def ollama_install_command(os_name: str, has: Callable[[str], bool]) -> "list[str] | None":
    """The package-manager install for macOS/Windows; None when no manager
    is present. Linux uses Ollama's own install script, which the wizard
    downloads and shows before running (see LINUX_INSTALL_SCRIPT)."""
    if os_name == "macos" and has("brew"):
        return ["brew", "install", "ollama"]
    if os_name == "windows" and has("winget"):
        return ["winget", "install", "--id", "Ollama.Ollama", "-e",
                "--accept-source-agreements", "--accept-package-agreements"]
    return None


def server_env(embedder: Embedder) -> dict[str, str]:
    """Environment a harness passes to a stdio velesdb-memory. The hash
    embedder is the server's default, so it needs none."""
    if embedder.backend == "hash":
        return {}
    return {"VELESDB_MEMORY_EMBEDDER": embedder.backend, "VELESDB_MEMORY_EMBEDDER_MODEL": embedder.model}
