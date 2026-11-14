"""Embedder choice for the install wizard, checked against the existing store.

A store is readable only by the embedding model that filled it:
- a different **dimension** is refused by velesdb-core when the collection
  opens (VELES-004);
- a different **model of the same dimension** opens and then recalls noise,
  which is why velesdb-memory records `embedding-provenance.json`
  (`crates/velesdb-memory/src/embedding_provenance.rs`) and refuses, before it
  opens the store, any configuration that disagrees with the record.

So the wizard never proposes a model blind: it reads the store, classifies
every candidate, defaults to the model the store records (rewiring a harness
must not silently switch it back to `hash`), probes the real dimension of a
pulled model, and runs `velesdb-memory migrate-embeddings`
(docs/guides/MIGRATE_EMBEDDINGS.md) only when the user agrees — switching
embedders never costs the memories.
"""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass, replace
from pathlib import Path
from typing import Callable

#: The built-in offline embedder: deterministic, not semantic, 384-dim.
HASH_DIMENSION = 384
COLLECTIONS = ("_semantic_memory", "_episodic_memory", "_procedural_memory")
PROVENANCE_FILE = "embedding-provenance.json"
LINUX_INSTALL_SCRIPT = "https://ollama.com/install.sh"
OLLAMA_URL = "http://localhost:11434"
DEFAULT_OLLAMA_MODEL = "all-minilm"

# compatibility verdicts
FRESH = "fresh"            # no store yet: it will be created at this model's dimension
MATCH = "match"            # the store records this model at this dimension
UNVERIFIED = "unverified"  # legacy store without a provenance record: only the dimension can be checked
MIGRATE = "migrate"        # needs `velesdb-memory migrate-embeddings` (or another store)
UNKNOWN = "unknown"        # dimension not known until the model is pulled and probed

#: Name fragments of embedding models, to keep chat models out of the menu.
#: A display filter only: --embedder-model accepts any name, and the probe
#: is what proves a model embeds.
_EMBEDDING_HINTS = ("embed", "bge", "minilm", "e5", "gte", "nomic", "mxbai", "arctic", "granite-embedding")


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
    dimension: "int | None"  # None until probed (a model outside the catalog)
    url: "str | None" = None  # an Ollama endpoint other than OLLAMA_URL


#: The server's endpoint variables and their legacy aliases (remote_endpoint.rs:
#: the EMBEDDER_ name wins and the pair is reported as a conflict).
_ALIASES = {
    "VELESDB_MEMORY_EMBEDDER_MODEL": "VELESDB_MEMORY_OLLAMA_MODEL",
    "VELESDB_MEMORY_EMBEDDER_URL": "VELESDB_MEMORY_OLLAMA_URL",
}


def resolve_url(flag: "str | None", environ: "dict[str, str]") -> "str | None":
    """--ollama-url, else the endpoint the server itself would read from this
    environment; None for the default.

    Raises ValueError for anything but an http(s) URL with a host: the wizard
    fetches it and writes it into every harness config and the daemon."""
    url = flag or environ.get("VELESDB_MEMORY_EMBEDDER_URL") or environ.get("VELESDB_MEMORY_OLLAMA_URL")
    url = (url or "").strip().rstrip("/")
    if url in ("", OLLAMA_URL):
        return None
    parts = urllib.parse.urlsplit(url)
    if parts.scheme not in ("http", "https") or not parts.hostname:
        raise ValueError(f"the Ollama endpoint must be an http(s) URL with a host, not {url!r}")
    return url


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
    dimension: "int | None"
    size: str
    note: str
    installed: bool
    verdict: str


# --------------------------------------------------------------------------
# The store and the verdicts


def _read_json(path: Path) -> "dict | None":
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    return data if isinstance(data, dict) else None


def read_store(store: Path) -> StoreProfile:
    configs = [c for c in (_read_json(store / name / "config.json") for name in COLLECTIONS) if c]
    if not configs:
        return StoreProfile(exists=False)
    provenance = _read_json(store / PROVENANCE_FILE) or {}
    dims = [c["dimension"] for c in configs if isinstance(c.get("dimension"), int)]
    return StoreProfile(
        exists=True,
        dimension=dims[0] if dims else provenance.get("dimension"),
        model=provenance.get("model"),
        has_facts=any(int(c.get("point_count") or 0) > 0 for c in configs),
    )


def compatibility(store: StoreProfile, model: str, dimension: "int | None") -> str:
    """Mirrors daemon_startup: the provenance check runs before the store is
    opened, so a recorded model that differs is refused even on an empty
    store; the core then refuses a dimension mismatch (VELES-004)."""
    if not store.exists:
        return FRESH
    if store.model is not None:
        return MATCH if store.model == model and dimension in (None, store.dimension) else MIGRATE
    if dimension is None:
        return UNKNOWN
    return UNVERIFIED if store.dimension == dimension else MIGRATE


def _bare(name: str) -> str:
    return name.split(":", 1)[0]


def _catalog_model(name: str) -> "Model | None":
    return next((m for m in CATALOG if m.name == _bare(name)), None)


def local_models(tags: dict) -> list[str]:
    """Model names from Ollama's GET /api/tags response."""
    return [str(m.get("name")) for m in tags.get("models", []) if isinstance(m, dict) and m.get("name")]


def _exact(name: str) -> str:
    """The name Ollama resolves the same way: `x:latest` is `x`, `x:335m` is not."""
    return _bare(name) if name.endswith(":latest") else name


def is_installed(name: str, local: "list[str]") -> bool:
    exact = {_exact(m) for m in local}
    return _exact(name) in exact


def _looks_like_embedder(name: str) -> bool:
    bare = _bare(name).lower()
    return any(hint in bare for hint in _EMBEDDING_HINTS)


def rank_models(store: StoreProfile, local: "list[str]") -> list[Candidate]:
    """Catalog models, then local embedding models outside it; usable ones
    first, installed before not."""
    candidates = [Candidate(m.name, m.dimension, m.size, m.note, is_installed(m.name, local),
                            compatibility(store, m.name, m.dimension)) for m in CATALOG]
    extra = sorted({_exact(m) for m in local if _looks_like_embedder(m) and _catalog_model(m) is None})
    candidates += [Candidate(name, None, "local", "installed in Ollama", True,
                             compatibility(store, name, None)) for name in extra]
    order = {MATCH: 0, FRESH: 0, UNVERIFIED: 1, UNKNOWN: 1, MIGRATE: 2}
    return sorted(candidates, key=lambda c: (order[c.verdict], not c.installed))


# --------------------------------------------------------------------------
# Which embedder


def default_embedder(store: StoreProfile) -> Embedder:
    """The model the store records — the only one that can read it — else
    the offline default. The backend is not recorded (a transport, not a
    model); a recorded non-hash model is assumed to be served by Ollama."""
    if store.model is None or store.model == HASH.model:
        return HASH
    known = _catalog_model(store.model)
    return Embedder("ollama", store.model, store.dimension or (known.dimension if known else None))


def select_embedder(backend: "str | None", model: "str | None", store: StoreProfile,
                    url: "str | None" = None) -> Embedder:
    """--embedder / --embedder-model, or the store's default; `url` is the
    Ollama endpoint (see resolve_url)."""
    chosen = _selection(backend, model, store)
    return chosen if chosen.backend == "hash" else replace(chosen, url=url)


def _selection(backend: "str | None", model: "str | None", store: StoreProfile) -> Embedder:
    if backend is None and model is None:
        return default_embedder(store)
    if backend == "hash":
        return HASH
    current = default_embedder(store)
    name = model or (current.model if current.backend == "ollama" else DEFAULT_OLLAMA_MODEL)
    known = _catalog_model(name)
    return Embedder("ollama", name, known.dimension if known else None)


def describe(embedder: Embedder, store: StoreProfile) -> dict:
    """The plan's `embedder` section."""
    return {
        "backend": embedder.backend,
        "model": embedder.model,
        "dimension": embedder.dimension,
        "url": embedder.url,
        "verdict": compatibility(store, embedder.model, embedder.dimension),
        "store_model": store.model,
        "store_dimension": store.dimension,
        "store_has_facts": store.has_facts,
    }


def from_plan(section: dict) -> Embedder:
    return Embedder(section["backend"], section["model"], section.get("dimension"), section.get("url"))


def server_env(embedder: Embedder) -> dict[str, str]:
    """Environment a harness passes to a stdio velesdb-memory, on top of
    VELESDB_MEMORY_PATH. Explicit even for the default `hash`, so an entry
    states what it was wired for."""
    if embedder.backend == "hash":
        return {"VELESDB_MEMORY_EMBEDDER": "hash"}
    env = {"VELESDB_MEMORY_EMBEDDER": embedder.backend, "VELESDB_MEMORY_EMBEDDER_MODEL": embedder.model}
    if embedder.url:
        env["VELESDB_MEMORY_EMBEDDER_URL"] = embedder.url
    return env


def owned_env_keys(env: "dict[str, str]") -> set[str]:
    """The variables a wizard-written `env` replaces in an existing entry:
    its own, their legacy aliases, and the model whatever the backend (a
    stale model under `hash` is a trap for the next switch back). Every
    other variable — an extractor, a TTL, a log filter — is the user's."""
    owned = set(env) | {"VELESDB_MEMORY_EMBEDDER_MODEL"}
    return owned | {_ALIASES[k] for k in owned if k in _ALIASES}


def daemon_flags(embedder: Embedder, os_name: str) -> list[str]:
    """install-memory-daemon flags. Always explicit: the wizard runs the
    installer without a terminal, where an unset embedder falls back to hash."""
    ollama = embedder.backend == "ollama"
    if os_name == "windows":
        flags = ["-Embedder", embedder.backend] + (["-OllamaModel", embedder.model] if ollama else [])
        return flags + (["-OllamaUrl", embedder.url] if ollama and embedder.url else [])
    flags = [f"--embedder={embedder.backend}"] + ([f"--ollama-model={embedder.model}"] if ollama else [])
    return flags + ([f"--ollama-url={embedder.url}"] if ollama and embedder.url else [])


def needs_migration(embedder: Embedder, store_dir: Path) -> bool:
    return compatibility(read_store(store_dir), embedder.model, embedder.dimension) == MIGRATE


def migration_cmd(binary: str, store: Path, stamp: "str | None" = None) -> list[str]:
    """Offline migration: rebuilt next to the store (same filesystem, the
    final switch is a rename), then switched into place. One destination per
    migration: a completed one leaves its journal beside its destination,
    and a rerun there is refused as already done. An interrupted run is
    resumed by rerunning the exact command, which the failure status gives."""
    stamp = stamp or time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    destination = store.with_name(f"{store.name}.migration-{stamp}")
    return [binary, "migrate-embeddings", "--store", str(store), "--destination", str(destination)]


# --------------------------------------------------------------------------
# Interactive choice


def _menu_line(index: int, c: Candidate) -> str:
    dim = f"{c.dimension}d" if c.dimension else "?d"
    tags = {MATCH: "this store's model", FRESH: "", UNVERIFIED: "dimension matches (unrecorded store)",
            UNKNOWN: "dimension probed after pull", MIGRATE: "store migration needed"}
    mark = "installed" if c.installed else f"download {c.size}"
    extra = f" — {tags[c.verdict]}" if tags[c.verdict] else ""
    return f"  [{index}] {c.name:<20} {dim:>6}  {c.note} ({mark}){extra}"


def choose_embedder(store: StoreProfile, local: "list[str]", ask: Callable[[str], str],
                    say: Callable[[str], None] = print) -> Embedder:
    """Numbered menu; Enter keeps the default (the store's model)."""
    default = default_embedder(store)
    candidates = rank_models(store, local)
    say("Embedding model (vectors are only comparable within one model):")
    say(f"  [0] {'hash':<20} {HASH_DIMENSION:>5}d  offline, deterministic, NOT semantic")
    for i, c in enumerate(candidates, 1):
        say(_menu_line(i, c))
    while True:
        answer = ask(f"Model [default: {default.model}]: ").strip()
        if not answer:
            return default
        if answer == "0":
            return HASH
        if answer.isdigit() and 1 <= int(answer) <= len(candidates):
            picked = candidates[int(answer) - 1]
            return Embedder("ollama", picked.name, picked.dimension)
        say("Invalid choice.")


# --------------------------------------------------------------------------
# Ollama: install, pull, probe — real I/O behind one seam for the tests


class Ops:
    """The side effects of the embedder step (Ollama and the migration)."""

    def __init__(self, url: str = OLLAMA_URL) -> None:
        self.url = url

    def has(self, command: str) -> bool:
        return shutil.which(command) is not None

    def run(self, cmd: "list[str]", env: "dict[str, str] | None" = None) -> int:
        try:
            return subprocess.run(cmd, env={**os.environ, **(env or {})}, stdin=subprocess.DEVNULL,
                                  timeout=7200).returncode
        except (OSError, subprocess.SubprocessError):
            return 127

    def _request(self, path: str, body: "dict | None" = None, timeout: float = 3.0) -> "dict | None":
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(self.url + path, data=data, headers={"Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=timeout) as response:  # noqa: S310 (localhost URL)
                parsed = json.loads(response.read().decode("utf-8"))
        except (OSError, ValueError, urllib.error.URLError):
            return None
        return parsed if isinstance(parsed, dict) else None

    def tags(self) -> "list[str] | None":
        """Installed models, or None when no Ollama answers."""
        reply = self._request("/api/tags")
        return None if reply is None else local_models(reply)

    def embed_dimension(self, model: str) -> "int | None":
        reply = self._request("/api/embed", {"model": model, "input": "dimension probe"}, timeout=120.0)
        return probed_dimension(reply)

    def fetch_installer(self) -> "Path | None":
        try:
            with urllib.request.urlopen(LINUX_INSTALL_SCRIPT, timeout=30) as response:  # noqa: S310
                body = response.read()
        except (OSError, urllib.error.URLError):
            return None
        handle, name = tempfile.mkstemp(prefix="ollama-install-", suffix=".sh")
        with os.fdopen(handle, "wb") as out:
            out.write(body)
        return Path(name)


def probed_dimension(reply: "dict | None") -> "int | None":
    """The width of the first vector of an Ollama /api/embed reply."""
    vectors = (reply or {}).get("embeddings")
    if isinstance(vectors, list) and vectors and isinstance(vectors[0], list) and vectors[0]:
        return len(vectors[0])
    return None


def ollama_install_command(os_name: str, has: Callable[[str], bool]) -> "list[str] | None":
    """The package-manager install for macOS/Windows; None when no manager
    is present. Linux uses Ollama's own install script (see install_ollama)."""
    if os_name == "macos" and has("brew"):
        return ["brew", "install", "ollama"]
    if os_name == "windows" and has("winget"):
        return ["winget", "install", "--id", "Ollama.Ollama", "-e",
                "--accept-source-agreements", "--accept-package-agreements"]
    return None


@dataclass
class Session:
    """What one Ollama step needs: the side effects, a yes/no question, the
    `--yes` flag and dry-run."""

    ops: Ops
    confirm: Callable[[str], bool]
    yes: bool = False
    dry: bool = False
    say: Callable[[str], None] = print

    def agree(self, question: str) -> bool:
        return self.yes or self.confirm(question)


def install_ollama(os_name: str, s: Session) -> str:
    """Install Ollama with the user's consent. Returns a status."""
    command = ollama_install_command(os_name, s.ops.has)
    if command is None and os_name == "linux":
        return _run_downloaded_installer(s)
    if command is None:
        return "ollama: FAILED — not installed and no package manager found; see https://ollama.com/download"
    if not s.agree(f"Install Ollama with `{' '.join(command)}`?"):
        return "ollama: REFUSED — not installed (declined; --yes to accept, --embedder=hash to stay offline)"
    return _installed_by(command, s)


def _run_downloaded_installer(s: Session) -> str:
    """Ollama's Linux install script. Unlike a package manager, nothing
    verifies what was downloaded, so `--yes` never runs it: only a person's
    explicit answer does. The downloaded copy is removed either way."""
    script = s.ops.fetch_installer()
    if script is None:
        return f"ollama: FAILED — could not download {LINUX_INSTALL_SCRIPT}"
    try:
        digest = hashlib.sha256(script.read_bytes()).hexdigest()
        s.say(f"Ollama's installer, downloaded to {script} (sha256 {digest[:16]}…) — read it before agreeing.")
        command = ["sh", str(script)]
        if not s.confirm(f"Install Ollama with `{' '.join(command)}`?"):
            return (f"ollama: REFUSED — a downloaded installer needs an explicit answer, never --yes; "
                    f"review and run {LINUX_INSTALL_SCRIPT} yourself, or --embedder=hash to stay offline")
        return _installed_by(command, s)
    finally:
        script.unlink(missing_ok=True)


def _installed_by(command: "list[str]", s: Session) -> str:
    if s.ops.run(command) != 0:
        return f"ollama: FAILED — `{' '.join(command)}` did not succeed"
    return "ollama: installed"


def _ensure_ollama(os_name: str, s: Session) -> "tuple[list[str] | None, list[str]]":
    local = s.ops.tags()
    if local is not None:
        return local, []
    if s.ops.has("ollama"):
        return None, ["ollama: FAILED — installed but not answering on " + s.ops.url + " (start it: `ollama serve`)"]
    status = install_ollama(os_name, s)
    if "FAILED" in status or "REFUSED" in status:
        return None, [status]
    local = s.ops.tags()
    if local is None:
        return None, [status, "ollama: FAILED — installed but not answering yet (start it: `ollama serve`)"]
    return local, [status]


def _ensure_model(embedder: Embedder, local: "list[str]", s: Session) -> "tuple[Embedder | None, list[str]]":
    if not is_installed(embedder.model, local):
        known = _catalog_model(embedder.model)
        size = f" ({known.size})" if known else ""
        if not s.agree(f"Download the embedding model {embedder.model}{size} with `ollama pull`?"):
            return None, [f"ollama: REFUSED — {embedder.model} not pulled (declined; --yes to accept)"]
        if s.ops.run(["ollama", "pull", embedder.model]) != 0:
            return None, [f"ollama: FAILED — `ollama pull {embedder.model}` did not succeed"]
    dimension = s.ops.embed_dimension(embedder.model)
    if dimension is None:
        return None, [f"ollama: FAILED — {embedder.model} returned no embedding (is it an embedding model?)"]
    if embedder.dimension not in (None, dimension):
        note = f"ollama: {embedder.model} embeds at {dimension}d, not the catalog's {embedder.dimension}d — using {dimension}"
        return replace(embedder, dimension=dimension), [note]
    return replace(embedder, dimension=dimension), [f"ollama: {embedder.model} ready ({dimension}d)"]


def prepare_ollama(embedder: Embedder, os_name: str, s: Session) -> "tuple[Embedder | None, list[str]]":
    """Ollama answering, the model pulled, its real dimension probed. None
    when the step failed or was declined."""
    if embedder.backend != "ollama":
        return embedder, []
    if s.dry:
        return embedder, [f"embedder: would check Ollama, pull {embedder.model} if absent, probe its dimension"]
    local, statuses = _ensure_ollama(os_name, s)
    if local is None:
        return None, statuses
    ready, more = _ensure_model(embedder, local, s)
    return ready, statuses + more


def reconcile_store(embedder: Embedder, store_dir: Path, binary: "str | None", s: Session,
                    locked: bool) -> "tuple[bool, list[str]]":
    """(ok to wire, statuses). A store the embedder cannot read is migrated
    with consent — never rewired over: the server would refuse to start."""
    store = read_store(store_dir)
    verdict = compatibility(store, embedder.model, embedder.dimension)
    if verdict != MIGRATE:
        return True, [f"embedder: {embedder.model} ({embedder.dimension or '?'}d) — store {verdict}"]
    recorded = f"{store.model or 'unrecorded'} ({store.dimension}d)"
    if s.dry:
        return True, [f"embedder: would migrate the store from {recorded} to {embedder.model} (migrate-embeddings)"]
    if locked:
        return False, [f"embedder: REFUSED — the store ({recorded}) needs migrate-embeddings but is locked by a "
                       "running process; stop it, or migrate online (docs/guides/MIGRATE_EMBEDDINGS.md)"]
    if binary is None:
        return False, ["embedder: REFUSED — the store needs migrate-embeddings and no velesdb-memory binary is on disk"]
    if not s.agree(f"The store holds vectors from {recorded}; re-embed it with {embedder.model}?"):
        return False, [f"embedder: REFUSED — store left on {recorded} (migration declined; --yes to accept)"]
    cmd = migration_cmd(binary, store_dir)
    code = s.ops.run(cmd, server_env(embedder))
    if code != 0:
        return False, [f"embedder: FAILED — migrate-embeddings exited {code}; the store is unchanged or resumable — "
                       f"resume with the same environment ({_env_line(embedder)}): {' '.join(cmd)}"]
    return True, [f"embedder: store migrated to {embedder.model}"]


def _env_line(embedder: Embedder) -> str:
    return " ".join(f"{k}={v}" for k, v in server_env(embedder).items())
