<#
.SYNOPSIS
    Automates VelesDB refactor PRD execution via Ralphy in 6 phases.
.DESCRIPTION
    1. Commits current workspace state (backup)
    2. Executes each Ralphy phase sequentially.
       - Uses --max-parallel 2 when safe
       - Generates branches + PRs automatically
    3. Logs progress to .ralphy/progress.txt
    Requires:
      - Ralphy CLI installed and configured with permission "allow"
      - Git credentials and GitHub PAT configured
      - Environment variable OPENROUTER_API_KEY (or relevant)
#>

$ErrorActionPreference = 'Stop'

function Log {
    param($Message)
    $ts = Get-Date -Format 'u'
    "$ts `t $Message" | Out-File -FilePath ".ralphy\progress.txt" -Append -Encoding UTF8
    Write-Host $Message -ForegroundColor Cyan
}

function Run-Phase {
    param(
        [string]$Name,
        [string]$Command
    )
    Log "=== PHASE $Name ==="
    Log "$Command"
    & bash -c "$Command" 2>&1 | Tee-Object -FilePath ".ralphy\progress.txt" -Append
    if ($LASTEXITCODE -ne 0) {
        Log "Phase $Name failed with code $LASTEXITCODE. Aborting batch."
        exit $LASTEXITCODE
    }
    Log "=== END PHASE $Name ==="
}

# 1. Backup commit ---------------------------
Log "Backup commit before Ralphy batch"
git add .
$commitMsg = "backup before Ralphy batch $(Get-Date -Format 'yyyy-MM-dd_HH-mm')"
git commit -m $commitMsg
if ($LASTEXITCODE -eq 0) { git push origin develop }

# 2. Phase execution -------------------------

# Phase 1
Run-Phase "1-Fondations" "ralphy --prd ./prd/01-utils-module.md --opencode --branch-per-task --create-pr --base-branch develop"

# Phase 2 (depends on merge of phase 1 PR). You may want manual validation, but we enqueue anyway.
Run-Phase "2-Refactor" "ralphy --prd ./prd/02-agent-refactor.md --opencode --branch-per-task --create-pr --base-branch develop"

# Phase 3 – SIMD & GPU in parallel ≤2
Run-Phase "3-Optim" "ralphy --prd ./prd/03-simd-arxiv.md ./prd/04-unsafe-miri.md ./prd/05-gpu-audit.md --opencode --branch-per-task --create-pr --base-branch develop --max-parallel 2"

# Phase 4 – Devin bugs
Run-Phase "4-Bugs" "ralphy --prd ./prd/06-devin-bugs.md --opencode --branch-per-task --create-pr --base-branch develop"

# Phase 5 – Docs & CI
Run-Phase "5-Docs" "ralphy --prd ./prd/07-docs-ci.md --opencode --branch-per-task --create-pr --base-branch develop"

# Phase 6 – Review finale
Run-Phase "6-Review" "ralphy --prd ./prd/08-final-review.md --opencode --branch-per-task --create-pr --base-branch develop"

Log "Batch completed successfully"
