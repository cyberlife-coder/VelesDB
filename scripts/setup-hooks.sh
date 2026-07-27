#!/usr/bin/env bash
# =============================================================================
# VelesDB - Git Hooks Setup (Linux / macOS)
# =============================================================================
# Point git at the project's hooks so pre-commit and pre-push validation runs
# locally, before CI has to say no.
#
# Usage: ./scripts/setup-hooks.sh
#
# The Windows equivalent is scripts/setup-hooks.ps1. Both do the same thing;
# this one additionally restores the executable bit, which a fresh clone can
# lose (zip download, `core.fileMode=false`, some CI checkouts) and which git
# silently ignores — an un-executable hook does not fail, it just never runs.
# =============================================================================

set -euo pipefail

BLUE=$'\033[0;34m'; CYAN=$'\033[0;36m'; GREEN=$'\033[0;32m'
YELLOW=$'\033[0;33m'; RED=$'\033[0;31m'; RESET=$'\033[0m'

# Run from the repository root whatever directory the user invoked this from.
cd "$(git rev-parse --show-toplevel)"

printf '\n%s\n' "${BLUE}═══════════════════════════════════════════════════════════════════${RESET}"
printf '%s\n'   "${BLUE}  VelesDB - Git Hooks Setup${RESET}"
printf '%s\n\n' "${BLUE}═══════════════════════════════════════════════════════════════════${RESET}"

if [ ! -d .githooks ]; then
  printf '%s\n' "${RED}❌ .githooks/ not found — are you in the VelesDB repository?${RESET}"
  exit 1
fi

printf '%s\n' "${CYAN}📋 Configuring git hooks path...${RESET}"
git config core.hooksPath .githooks

configured=$(git config --get core.hooksPath || true)
if [ "$configured" != ".githooks" ]; then
  printf '%s\n' "${RED}❌ Failed to configure git hooks (core.hooksPath is '${configured}')${RESET}"
  exit 1
fi

printf '%s\n\n' "${GREEN}✅ Git hooks configured — hooks directory: .githooks/${RESET}"

# Restore the executable bit where it is missing. git records it, but a
# checkout with core.fileMode=false (or an archive extraction) can drop it.
restored=0
for hook in .githooks/*; do
  [ -f "$hook" ] || continue
  if [ ! -x "$hook" ]; then
    chmod +x "$hook"
    restored=$((restored + 1))
  fi
done
if [ "$restored" -gt 0 ]; then
  printf '%s\n\n' "${YELLOW}🔧 Restored the executable bit on ${restored} hook(s).${RESET}"
fi

printf '%s\n' "${CYAN}📌 Active hooks:${RESET}"
declare -a described=()
[ -f .githooks/commit-msg ] && described+=("   - commit-msg  : rejects AI-attributed authors and AI attribution trailers")
[ -f .githooks/pre-commit ] && described+=("   - pre-commit  : validates the change before each commit")
[ -f .githooks/pre-push ]   && described+=("   - pre-push    : full local validation before pushing to origin")
if [ ${#described[@]} -eq 0 ]; then
  printf '%s\n' "${RED}   (none found in .githooks/ — nothing will run)${RESET}"
  exit 1
fi
printf '%s\n' "${described[@]}"

printf '\n%s\n' "${YELLOW}💡 Workflow:${RESET}"
printf '%s\n' "   1. Make your changes"
printf '%s\n' "   2. git commit -m '...'      # commit-msg + pre-commit run"
printf '%s\n' "   3. git push origin <branch> # pre-push runs the full local gate"

printf '\n%s\n\n' "${BLUE}═══════════════════════════════════════════════════════════════════${RESET}"
