"""Validate claims copied across multiple documentation surfaces."""

from __future__ import annotations

import pathlib


def _normalized(value: object) -> str:
    return " ".join(str(value).split())


def _required_text(item: dict, field: str, owner: str) -> tuple[str, list[str]]:
    value = item.get(field)
    if not isinstance(value, str) or not value.strip():
        return "", [f"[{owner}] missing non-empty '{field}'"]
    return value.strip(), []


def _member_failures(
    family_id: str,
    canonical_value: str,
    member: dict,
    root: pathlib.Path,
) -> list[str]:
    rel_path, failures = _required_text(member, "file", family_id)
    value, value_failures = _required_text(member, "value", family_id)
    needle, needle_failures = _required_text(member, "must_contain", family_id)
    failures.extend(value_failures + needle_failures)
    if failures:
        return failures
    path = pathlib.PurePosixPath(rel_path)
    if path.is_absolute() or ".." in path.parts:
        return [f"[{family_id}] member path must stay inside the repository: {rel_path}"]
    if _normalized(value) != _normalized(canonical_value):
        return [
            f"[{family_id}] {rel_path} publishes {value!r}; "
            f"canonical value is {canonical_value!r}"
        ]
    if _normalized(value) not in _normalized(needle):
        return [f"[{family_id}] {rel_path} member substring omits its value: {needle!r}"]
    file_path = root / rel_path
    if not file_path.exists():
        return [f"[{family_id}] missing member file: {rel_path}"]
    if needle not in file_path.read_text(encoding="utf-8"):
        return [
            f"[{family_id}] expected substring not found in {rel_path}: {needle!r}"
        ]
    return []


def _family_failures(family: dict, claims: dict[str, dict], root: pathlib.Path) -> list[str]:
    family_id, failures = _required_text(family, "id", "claim-family")
    canonical_id, canonical_failures = _required_text(
        family, "canonical_claim_id", family_id or "claim-family"
    )
    canonical_value, value_failures = _required_text(
        family, "canonical_value", family_id or "claim-family"
    )
    failures.extend(canonical_failures + value_failures)
    members = family.get("members")
    if failures:
        return failures
    if canonical_id not in claims:
        return [f"[{family_id}] unknown canonical claim: {canonical_id}"]
    canonical_needle = claims[canonical_id].get("must_contain", "")
    if _normalized(canonical_value) not in _normalized(canonical_needle):
        return [f"[{family_id}] canonical claim does not publish {canonical_value!r}"]
    if not isinstance(members, list) or len(members) < 2:
        return [f"[{family_id}] family must declare at least two members"]
    member_files = {member.get("file") for member in members if isinstance(member, dict)}
    if len(member_files) < 2:
        return [f"[{family_id}] family must propagate across two distinct files"]
    result = []
    for member in members:
        if not isinstance(member, dict):
            result.append(f"[{family_id}] every member must be an object")
            continue
        result.extend(_member_failures(family_id, canonical_value, member, root))
    return result


def check_claim_families(data: dict, root: pathlib.Path) -> list[str]:
    """Validate family schema, canonical values, and every declared occurrence."""
    families = data.get("claim_families")
    if not isinstance(families, list) or not families:
        return ["Registry has no claim_families"]
    claims = {
        claim.get("id"): claim
        for claim in data.get("claims", [])
        if isinstance(claim, dict) and claim.get("id")
    }
    failures = []
    seen = set()
    for family in families:
        if not isinstance(family, dict):
            failures.append("Every claim family must be an object")
            continue
        family_id = family.get("id")
        if family_id in seen:
            failures.append(f"Duplicate claim family id: {family_id}")
            continue
        seen.add(family_id)
        failures.extend(_family_failures(family, claims, root))
    return failures
