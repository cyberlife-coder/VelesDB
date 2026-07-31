#!/bin/bash
# =============================================================================
# VelesDB Python Integration Checks
# =============================================================================
# Checks for common security and correctness issues in Python integrations:
# - Dangerous hash() usage (non-deterministic across processes)
# - Security issues (bandit)
# - Type hints (mypy optional)
# =============================================================================

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

PYTHON_DIRS="integrations/langchain integrations/llamaindex"
ERRORS=0

echo -e "${YELLOW}🐍 VelesDB Python Integration Checks...${NC}\n"

# =============================================================================
# Check 1: Dangerous hash() patterns
# =============================================================================
echo -e "${YELLOW}1️⃣  Checking for dangerous hash() usage...${NC}"

# Pattern: hash() used for IDs, deduplication, or persistence
# This is non-deterministic across Python processes (PYTHONHASHSEED)
DANGEROUS_HASH=$(grep -rn "hash(" $PYTHON_DIRS --include="*.py" 2>/dev/null | \
    grep -v "hashlib" | \
    grep -v "#.*hash" | \
    grep -v "def.*hash" | \
    grep -v "__hash__" | \
    grep -v "test_" | \
    grep -v '"""' | \
    grep -v "'''" | \
    grep -v "Python's hash" | \
    grep -v "hash()" || true)

if [ -n "$DANGEROUS_HASH" ]; then
    echo -e "${RED}❌ Dangerous hash() usage detected:${NC}"
    echo "$DANGEROUS_HASH"
    echo -e "${YELLOW}   Use hashlib.sha256() for deterministic hashing.${NC}"
    ERRORS=$((ERRORS + 1))
else
    echo -e "${GREEN}✅ No dangerous hash() patterns${NC}"
fi

# =============================================================================
# Check 2: Other non-deterministic patterns
# =============================================================================
echo -e "\n${YELLOW}2️⃣  Checking for non-deterministic patterns...${NC}"

# Pattern: id() used for persistence (memory address, not stable)
ID_USAGE=$(grep -rn "\bid(" $PYTHON_DIRS --include="*.py" 2>/dev/null | \
    grep -v "node_id" | \
    grep -v "doc_id" | \
    grep -v "edge_id" | \
    grep -v "#" | \
    grep -v "def " || true)

if [ -n "$ID_USAGE" ]; then
    echo -e "${YELLOW}⚠️  Potential id() usage (review manually):${NC}"
    echo "$ID_USAGE"
else
    echo -e "${GREEN}✅ No suspicious id() patterns${NC}"
fi

# =============================================================================
# Check 3: Security linting with bandit (if available)
# =============================================================================
echo -e "\n${YELLOW}3️⃣  Running security checks...${NC}"

if command -v bandit &> /dev/null; then
    for dir in $PYTHON_DIRS; do
        # Guard the directory bandit is actually GIVEN. The test was on "$dir"
        # while the scan was on "$dir/src", so an integration laid out without a
        # src/ made bandit error out and counted as a finding — a refusal the
        # tree never earned.
        if [ -d "$dir/src" ]; then
            echo -e "   Scanning $dir..."
            if ! bandit -r "$dir/src" -ll -q 2>/dev/null; then
                ERRORS=$((ERRORS + 1))
            fi
        fi
    done
    echo -e "${GREEN}✅ Security scan complete${NC}"
else
    echo -e "${YELLOW}⚠️  bandit not installed, skipping security scan${NC}"
    echo -e "   Install with: pip install bandit"
fi

# =============================================================================
# Check 4: Cyclomatic complexity (flake8 + mccabe, CC <= 8)
# =============================================================================
echo -e "\n${YELLOW}4️⃣  Checking cyclomatic complexity (CC <= 8)...${NC}"

# Only hand flake8 directories that exist. `integrations/common` used to be
# passed unconditionally, and on a tree without it flake8 answers E902 on
# stdout — which --select=C901 does not suppress, so it counted as a complexity
# finding no function had caused.
COMPLEXITY_TARGETS=""
for dir in $PYTHON_DIRS integrations/common; do
    if [ -d "$dir" ]; then
        COMPLEXITY_TARGETS="$COMPLEXITY_TARGETS $dir"
    fi
done

if ! command -v flake8 &> /dev/null; then
    echo -e "${YELLOW}⚠️  flake8 not installed, skipping complexity check${NC}"
    echo -e "   Install with: pip install flake8 mccabe"
elif [ -z "$COMPLEXITY_TARGETS" ]; then
    echo -e "${YELLOW}⚠️  no Python integration directory present, nothing to measure${NC}"
else
    COMPLEXITY_ISSUES=$(flake8 $COMPLEXITY_TARGETS --select=C901 --max-complexity=8 2>/dev/null || true)
    if [ -n "$COMPLEXITY_ISSUES" ]; then
        echo -e "${RED}❌ Functions exceeding complexity threshold (CC > 8):${NC}"
        echo "$COMPLEXITY_ISSUES"
        ERRORS=$((ERRORS + 1))
    else
        echo -e "${GREEN}✅ All functions within complexity limit (CC <= 8)${NC}"
    fi
fi

# =============================================================================
# Check 5: Import verification
# =============================================================================
echo -e "\n${YELLOW}5️⃣  Verifying imports...${NC}"

# Check that hashlib is imported where needed
for dir in $PYTHON_DIRS; do
    if [ -d "$dir/src" ]; then
        # Find files that might need hashlib but don't import it
        FILES_WITH_SHA=$(grep -rl "sha256\|sha1\|md5" "$dir/src" --include="*.py" 2>/dev/null || true)
        for f in $FILES_WITH_SHA; do
            if ! grep -q "import hashlib" "$f"; then
                echo -e "${YELLOW}⚠️  $f uses hash functions but missing 'import hashlib'${NC}"
            fi
        done
    fi
done
echo -e "${GREEN}✅ Import check complete${NC}"

# =============================================================================
# Summary
# =============================================================================
echo -e "\n${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
if [ $ERRORS -gt 0 ]; then
    echo -e "${RED}❌ Python checks failed with $ERRORS error(s)${NC}"
    exit 1
else
    echo -e "${GREEN}🎉 All Python checks passed!${NC}"
fi
