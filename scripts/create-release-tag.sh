#!/usr/bin/env bash
# Create one guarded annotated release tag, then push that exact ref.

set -euo pipefail

readonly WORKSPACE_TAG_PATTERN='^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z][0-9A-Za-z.-]*)?$'
readonly MEMORY_TAG_PATTERN='^velesdb-memory-v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z][0-9A-Za-z.-]*)?$'
readonly SHA_PATTERN='^[0-9A-Fa-f]{40}$'

die() {
  echo "create-release-tag: $*" >&2
  exit 1
}

# The two release trains tag two different branches: the workspace ships from
# main, velesdb-memory from develop on its own 0.x cadence. The tag itself says
# which train it belongs to, so no caller can pair a tag with the wrong branch.
release_branch_for_tag() {
  local tag=$1

  if [[ "$tag" =~ $WORKSPACE_TAG_PATTERN ]]; then
    echo main
  elif [[ "$tag" =~ $MEMORY_TAG_PATTERN ]]; then
    echo develop
  else
    return 1
  fi
}

validate_inputs() {
  local tag=$1
  local sha=$2
  local message=$3

  release_branch_for_tag "$tag" >/dev/null \
    || die "tag '$tag' must match vX.Y.Z or velesdb-memory-vX.Y.Z, optionally with a -prerelease suffix"
  [[ "$sha" =~ $SHA_PATTERN ]] || die "sha must be a full 40-character commit SHA"
  [[ -n "$message" ]] || die "message must not be empty"
}

resolve_release_commit() {
  local sha=$1
  local branch=$2

  git fetch --no-tags origin "+refs/heads/${branch}:refs/remotes/origin/${branch}"
  git cat-file -e "${sha}^{commit}" 2>/dev/null || die "sha '$sha' is not a commit"
  git rev-parse --verify "${sha}^{commit}"
}

ensure_release_ancestor() {
  local sha=$1
  local branch=$2

  git merge-base --is-ancestor "$sha" "origin/${branch}" \
    || die "sha '$sha' is not an ancestor of origin/${branch}"
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
  local branch
  local resolved_sha

  validate_inputs "$tag" "$requested_sha" "$message"
  branch=$(release_branch_for_tag "$tag")
  resolved_sha=$(resolve_release_commit "$requested_sha" "$branch")
  ensure_release_ancestor "$resolved_sha" "$branch"
  ensure_tag_absent "$tag"
  create_and_push_tag "$tag" "$resolved_sha" "$message"
  echo "Created and pushed annotated tag $tag at $resolved_sha (origin/$branch)"
}

main "$@"
