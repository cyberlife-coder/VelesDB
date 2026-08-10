#!/usr/bin/env bash
# Create one guarded annotated release tag, then push that exact ref.

set -euo pipefail

readonly TAG_PATTERN='^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z][0-9A-Za-z.-]*)?$'
readonly SHA_PATTERN='^[0-9A-Fa-f]{40}$'

die() {
  echo "create-release-tag: $*" >&2
  exit 1
}

validate_inputs() {
  local tag=$1
  local sha=$2
  local message=$3

  [[ "$tag" =~ $TAG_PATTERN ]] || die "tag '$tag' must match vX.Y.Z or vX.Y.Z-prerelease"
  [[ "$sha" =~ $SHA_PATTERN ]] || die "sha must be a full 40-character commit SHA"
  [[ -n "$message" ]] || die "message must not be empty"
}

resolve_main_commit() {
  local sha=$1

  git fetch --no-tags origin '+refs/heads/main:refs/remotes/origin/main'
  git cat-file -e "${sha}^{commit}" 2>/dev/null || die "sha '$sha' is not a commit"
  git rev-parse --verify "${sha}^{commit}"
}

ensure_main_ancestor() {
  local sha=$1

  git merge-base --is-ancestor "$sha" origin/main \
    || die "sha '$sha' is not an ancestor of origin/main"
}

ensure_tag_absent() {
  local tag=$1
  local remote_refs

  git show-ref --verify --quiet "refs/tags/$tag" \
    && die "tag '$tag' already exists locally"
  remote_refs=$(git ls-remote --tags origin "refs/tags/$tag" "refs/tags/$tag^{}") \
    || die "unable to query tags on origin"
  [[ -z "$remote_refs" ]] || die "tag '$tag' already exists on origin"
}

create_and_push_tag() {
  local tag=$1
  local sha=$2
  local message=$3

  git tag --annotate "$tag" "$sha" --message "$message"
  if ! git push origin "refs/tags/$tag:refs/tags/$tag"; then
    git tag --delete "$tag" >/dev/null
    die "failed to push tag '$tag'"
  fi
}

main() {
  [[ $# -eq 3 ]] || die "usage: create-release-tag.sh <tag> <sha> <message>"
  local tag=$1
  local requested_sha=$2
  local message=$3
  local resolved_sha

  validate_inputs "$tag" "$requested_sha" "$message"
  resolved_sha=$(resolve_main_commit "$requested_sha")
  ensure_main_ancestor "$resolved_sha"
  ensure_tag_absent "$tag"
  create_and_push_tag "$tag" "$resolved_sha" "$message"
  echo "Created and pushed annotated tag $tag at $resolved_sha"
}

main "$@"
