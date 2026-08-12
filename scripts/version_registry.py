"""Read the public registries for the independently versioned memory packages."""

from __future__ import annotations

import json
from collections.abc import Callable
from typing import Any
from urllib.parse import quote
from urllib.request import Request, urlopen

CRATES_URL = "https://crates.io/api/v1/crates/velesdb-memory"
NPM_REGISTRY = "https://registry.npmjs.org"
NPM_PACKAGES = (
    "@wiscale/velesdb-memory-node",
    "@wiscale/velesdb-memory-node-darwin-arm64",
    "@wiscale/velesdb-memory-node-darwin-x64",
    "@wiscale/velesdb-memory-node-linux-arm64-gnu",
    "@wiscale/velesdb-memory-node-linux-x64-gnu",
    "@wiscale/velesdb-memory-node-win32-x64-msvc",
)

JsonObject = dict[str, Any]
JsonFetcher = Callable[[str], JsonObject]


class RegistryError(RuntimeError):
    """The registry could not provide trustworthy version metadata."""


def _fetch_json(url: str) -> JsonObject:
    request = Request(url, headers={"User-Agent": "VelesDB-version-sync/1"})
    try:
        with urlopen(request, timeout=15) as response:
            payload = json.load(response)
    except (OSError, json.JSONDecodeError) as exc:
        raise RegistryError(f"Could not query {url}: {exc}") from exc
    if not isinstance(payload, dict):
        raise RegistryError(f"Registry response from {url} is not a JSON object")
    return payload


def _field(payload: JsonObject, path: tuple[str, ...], source: str) -> str:
    value: Any = payload
    for key in path:
        if not isinstance(value, dict) or key not in value:
            dotted = ".".join(path)
            raise RegistryError(f"Missing {dotted} in registry response from {source}")
        value = value[key]
    if not isinstance(value, str) or not value:
        dotted = ".".join(path)
        raise RegistryError(f"Invalid {dotted} in registry response from {source}")
    return value


def _npm_url(package: str) -> str:
    return f"{NPM_REGISTRY}/{quote(package, safe='@')}"


def read_public_memory_versions(
    fetch_json: JsonFetcher | None = None,
) -> list[tuple[str, str]]:
    """Return every public version needed by a working memory installation."""
    fetch = fetch_json or _fetch_json
    crates = fetch(CRATES_URL)
    versions = [
        ("crates.io velesdb-memory", _field(crates, ("crate", "max_version"), CRATES_URL)),
    ]
    for package in NPM_PACKAGES:
        url = _npm_url(package)
        payload = fetch(url)
        latest = _field(payload, ("dist-tags", "latest"), url)
        versions.append((f"npm {package} latest", latest))
    return versions


def memory_registry_mismatches(
    expected: str,
    fetch_json: JsonFetcher | None = None,
) -> list[str]:
    """Describe public artifacts that do not match the memory manifest."""
    versions = read_public_memory_versions(fetch_json)
    return [
        f"{label}: expected {expected}, found {actual}"
        for label, actual in versions
        if actual != expected
    ]
