#!/bin/sh
# =============================================================================
# VelesDB install wizard — one-line installer for Linux and macOS
# =============================================================================
#   curl -fsSL https://raw.githubusercontent.com/cyberlife-coder/VelesDB/main/scripts/install-wizard.sh | sh
#
# Wizard flags go after `-s --`:
#   curl -fsSL …/install-wizard.sh | sh -s -- --dry-run
#
# Finds the newest velesdb-memory-vX.Y.Z release that carries a wizard for
# this machine (velesdb-install-wizard-<target>.tar.gz, built by
# release-memory.yml), verifies it against its .sha256 sidecar, and runs it.
# A machine with no native build (Intel macOS, other architectures) gets the
# source bundle (velesdb-install-wizard-src.tar.gz), run with python3 >= 3.9.
# The wizard itself is transient: it runs from a temporary directory, and the
# velesdb-memory it installs lands in ~/.local/bin.
#
# The checksum proves transfer integrity, not authenticity: the sidecar comes
# from the same release. The wizard also carries a publisher signature, which
# scripts/validate_package.py reads.
# =============================================================================
set -eu

REPO="cyberlife-coder/VelesDB"
API="https://api.github.com/repos/$REPO/releases?per_page=100"
DOWNLOAD="https://github.com/$REPO/releases/download"
SOURCE_ASSET="velesdb-install-wizard-src.tar.gz"
#: How many velesdb-memory releases to look back through for a wizard.
MAX_TAGS=10

say() { printf '%s\n' "velesdb: $*" >&2; }
die() { say "$*"; exit 1; }

# The target triple of release-memory.yml's build-wizard matrix, or nothing.
native_target() {
  case "$(uname -s) $(uname -m)" in
    "Linux x86_64"|"Linux amd64") echo "x86_64-unknown-linux-gnu" ;;
    "Linux aarch64"|"Linux arm64") echo "aarch64-unknown-linux-gnu" ;;
    "Darwin arm64"|"Darwin aarch64") echo "aarch64-apple-darwin" ;;
    *) echo "" ;;
  esac
}

# memory_tags LISTING: velesdb-memory-vX.Y.Z tags of a downloaded release
# listing, newest first (GitHub lists releases that way; a suffixed tag is a
# prerelease and is skipped). Downloaded first, not piped: POSIX sh has no
# pipefail, and a failed listing must not read as "no wizard released".
memory_tags() {
  grep -o '"tag_name": *"velesdb-memory-v[0-9]*\.[0-9]*\.[0-9]*"' "$1" \
    | sed 's/.*"\(velesdb-memory-v[^"]*\)"/\1/' \
    | head -n "$MAX_TAGS"
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

# fetch_verified TAG ASSET DIR: download ASSET and its sidecar into DIR and
# check the hash. 1 when the release has no such asset; dies on a mismatch.
fetch_verified() {
  base="$DOWNLOAD/$1/$2"
  curl -fsSL --max-time 120 -o "$3/$2" "$base" 2>/dev/null || return 1
  curl -fsSL --max-time 20 -o "$3/$2.sha256" "$base.sha256" 2>/dev/null \
    || die "$2 in $1 has no .sha256 sidecar — refusing to run an unverified wizard"
  expected="$(awk '{print $1}' "$3/$2.sha256")"
  actual="$(sha256_of "$3/$2")"
  [ "$expected" = "$actual" ] \
    || die "sha256 mismatch for $2 in $1 (expected $expected, got $actual) — the download may be corrupt or tampered"
}

has_python39() {
  command -v python3 >/dev/null 2>&1 \
    && python3 -c 'import sys; sys.exit(0 if sys.version_info >= (3, 9) else 1)' 2>/dev/null
}

# The interactive menu needs a terminal, and `curl | sh` gives the wizard the
# script as stdin: hand it the controlling terminal when there is one.
run_wizard() {
  if [ -t 0 ] || ! (exec </dev/tty) 2>/dev/null; then
    "$@"
  else
    "$@" </dev/tty
  fi
}

main() {
  command -v curl >/dev/null 2>&1 || die "curl is required"
  command -v tar >/dev/null 2>&1 || die "tar is required"
  target="$(native_target)"
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' EXIT INT TERM
  curl -fsSL --max-time 20 -o "$work/releases.json" "$API" || die "could not list the releases of $REPO"
  tags="$(memory_tags "$work/releases.json")"
  for tag in $tags; do
    if [ -n "$target" ] && fetch_verified "$tag" "velesdb-install-wizard-$target.tar.gz" "$work"; then
      say "running the install wizard from $tag ($target, sha256 OK)"
      tar -xzf "$work/velesdb-install-wizard-$target.tar.gz" -C "$work"
      run_wizard "$work/velesdb-install-wizard" "$@"
      return
    fi
    if has_python39 && fetch_verified "$tag" "$SOURCE_ASSET" "$work"; then
      say "running the install wizard from $tag (source bundle, python3, sha256 OK)"
      tar -xzf "$work/$SOURCE_ASSET" -C "$work"
      run_wizard python3 "$work/velesdb-install-wizard/scripts/velesdb_install_wizard.py" "$@"
      return
    fi
  done
  die "no release of $REPO carries a wizard for $(uname -s)/$(uname -m) yet. From a checkout: python3 scripts/velesdb_install_wizard.py"
}

main "$@"
