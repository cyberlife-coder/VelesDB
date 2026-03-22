# =============================================================================
# VelesDB-Core - Local CI Validation Script
# =============================================================================
# Ce script reproduit les vérifications CI en local AVANT tout push vers origin.
# Objectif: Réduire les coûts GitHub Actions en validant localement.
#
# Usage: .\scripts\local-ci.ps1 [-Quick] [-SkipTests] [-SkipSecurity] [-Miri]
#
# Options:
#   -Quick       : Mode rapide (fmt + clippy uniquement)
#   -SkipTests   : Sauter les tests
#   -SkipSecurity: Sauter l'audit de sécurité
#   -Miri        : Activer les tests Miri (EPIC-032/US-004, nécessite nightly)
# =============================================================================

param(
    [switch]$Quick,
    [switch]$SkipTests,
    [switch]$SkipSecurity,
    [switch]$Miri
)

$ErrorActionPreference = "Stop"

# Colors
function Write-Step { param($msg) Write-Host "`n📋 $msg" -ForegroundColor Cyan }
function Write-Success { param($msg) Write-Host "✅ $msg" -ForegroundColor Green }
function Write-Fail { param($msg) Write-Host "❌ $msg" -ForegroundColor Red }
function Write-Warn { param($msg) Write-Host "⚠️  $msg" -ForegroundColor Yellow }

$startTime = Get-Date
$errors = @()

Write-Host "`n" -NoNewline
Write-Host "═══════════════════════════════════════════════════════════════════" -ForegroundColor Blue
Write-Host "  VelesDB-Core - Local CI Validation" -ForegroundColor Blue
Write-Host "  Mode: $(if ($Quick) { 'QUICK' } else { 'FULL' })" -ForegroundColor Blue
Write-Host "═══════════════════════════════════════════════════════════════════" -ForegroundColor Blue

# ============================================================================
# Check 1: Formatting
# ============================================================================
Write-Step "Check 1: Formatting (rustfmt)"
try {
    cargo fmt --all -- --check
    Write-Success "Formatting OK"
} catch {
    Write-Fail "Formatting failed!"
    Write-Host "   Run: cargo fmt --all" -ForegroundColor Yellow
    $errors += "Formatting"
}

# ============================================================================
# Check 2: Linting (Clippy)
# ============================================================================
Write-Step "Check 2: Linting (clippy)"
try {
    cargo clippy --all-targets --all-features -- -D warnings -D clippy::pedantic 2>&1 | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "Clippy failed" }
    Write-Success "Clippy OK"
} catch {
    Write-Fail "Clippy found issues!"
    $errors += "Clippy"
}

Write-Step "Check 3: Clippy undocumented unsafe blocks"
try {
    cargo clippy -p velesdb-core --lib --bins -- -A warnings -D clippy::undocumented_unsafe_blocks 2>&1 | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "Undocumented unsafe blocks check failed" }
    Write-Success "Undocumented unsafe blocks OK"
} catch {
    Write-Fail "Undocumented unsafe blocks check failed!"
    $errors += "UndocumentedUnsafe"
}

# ============================================================================
# Check 3: Unsafe SAFETY template
# ============================================================================
Write-Step "Check 4: Unsafe SAFETY template"
try {
    $changedFiles = @(
        git diff --name-only --diff-filter=ACM
        git diff --cached --name-only --diff-filter=ACM
    ) | Where-Object { $_ } | Sort-Object -Unique

    $unsafeFiles = $changedFiles |
        Where-Object {
            $_ -match '^crates/velesdb-core/src/.*\.rs$' -and
            $_ -notmatch '/tests/' -and
            $_ -notmatch '/benches/' -and
            $_ -notmatch '_tests?\.rs$'
        }

    if ($unsafeFiles.Count -gt 0) {
        python scripts/verify_unsafe_safety_template.py --files $unsafeFiles --strict 2>&1 | Out-Host
        if ($LASTEXITCODE -ne 0) { throw "Unsafe SAFETY template check failed" }
    } else {
        Write-Host "   No changed production Rust files to validate." -ForegroundColor Yellow
    }
    Write-Success "Unsafe SAFETY template OK"
} catch {
    Write-Fail "Unsafe SAFETY template check failed!"
    $errors += "Unsafe-SAFETY"
}

# ============================================================================
# Check 4: TODO/FIXME/HACK governance
# ============================================================================
Write-Step "Check 5: TODO/FIXME/HACK governance"
try {
    $changedFiles = @(
        git diff --name-only --diff-filter=ACM
        git diff --cached --name-only --diff-filter=ACM
    ) | Where-Object { $_ } | Sort-Object -Unique

    $todoFiles = $changedFiles |
        Where-Object {
            $_ -match '^crates/velesdb-core/src/.*\.rs$' -and
            $_ -notmatch '/tests/' -and
            $_ -notmatch '/benches/' -and
            $_ -notmatch '_tests?\.rs$'
        }

    if ($todoFiles.Count -gt 0) {
        python scripts/check-todo-annotations.py --files $todoFiles 2>&1 | Out-Host
        if ($LASTEXITCODE -ne 0) { throw "TODO/FIXME/HACK governance failed" }
    } else {
        Write-Host "   No changed production Rust files to validate." -ForegroundColor Yellow
    }
    Write-Success "TODO/FIXME/HACK governance OK"
} catch {
    Write-Fail "TODO/FIXME/HACK governance failed!"
    $errors += "TODO-Governance"
}

# ============================================================================
# Check: Codacy CLI static analysis (WSL required)
# ============================================================================
Write-Step "Check: Codacy CLI static analysis"
try {
    $wslCheck = wsl -- bash -c "which codacy-cli 2>/dev/null" 2>&1
    if ($LASTEXITCODE -eq 0) {
        $codacyOutput = wsl -- bash -c "cd /mnt/d/Projets-dev/velesDB/velesdb-core && codacy-cli analyze 2>&1"
        if ($LASTEXITCODE -ne 0) { throw "Codacy CLI failed" }
        $findingsLine = $codacyOutput | Select-String "findings\." | Select-Object -Last 1
        if ($findingsLine -match "0 findings") {
            Write-Success "Codacy CLI: 0 findings"
        } else {
            Write-Fail "Codacy CLI found issues:"
            Write-Output $findingsLine
            $errors += "CodacyCLI"
        }
    } else {
        Write-Warn "Codacy CLI not installed in WSL - skipping"
        Write-Output "   Install: https://docs.codacy.com/related-tools/local-analysis/client-side-tools/"
    }
} catch {
    Write-Fail "Codacy CLI analysis failed!"
    $errors += "CodacyCLI"
}

# ============================================================================
# Check: Rust cyclomatic complexity (cargo-complexity, CC <= 8)
# ============================================================================
Write-Step "Check: Rust cyclomatic complexity (CC <= 8)"
try {
    $complexityCheck = cargo complexity --version 2>&1
    if ($LASTEXITCODE -eq 0) {
        $complexityOutput = cargo complexity --max-complexity 8 --format json crates/velesdb-core/src/ 2>&1
        if ($LASTEXITCODE -ne 0) {
            Write-Fail "Functions exceeding CC > 8 detected!"
            Write-Output $complexityOutput
            $errors += "RustComplexity"
        } else {
            Write-Success "Rust cyclomatic complexity OK (all functions CC <= 8)"
        }
    } else {
        Write-Warn "cargo-complexity not installed - skipping"
        Write-Output "   Install with: cargo install cargo-complexity"
    }
} catch {
    Write-Warn "cargo-complexity check skipped (not installed)"
    Write-Output "   Install with: cargo install cargo-complexity"
}

# ============================================================================
# Check: Python cyclomatic complexity (flake8 + mccabe, CC <= 8)
# ============================================================================
Write-Step "Check: Python cyclomatic complexity (CC <= 8)"
try {
    $flake8Check = python -m flake8 --version 2>&1
    if ($LASTEXITCODE -eq 0) {
        $flake8Output = python -m flake8 integrations/ scripts/ --select=C901 --max-complexity=8 2>&1
        if ($LASTEXITCODE -ne 0) {
            Write-Fail "Python functions exceeding CC > 8:"
            Write-Output $flake8Output
            $errors += "PythonComplexity"
        } else {
            Write-Success "Python cyclomatic complexity OK (all functions CC <= 8)"
        }
    } else {
        Write-Warn "flake8 not installed - skipping Python complexity check"
        Write-Output "   Install with: pip install flake8 mccabe"
    }
} catch {
    Write-Warn "Python complexity check skipped (flake8 not installed)"
    Write-Output "   Install with: pip install flake8 mccabe"
}

if ($Quick) {
    Write-Warn "Quick mode - skipping tests and security audit"
} else {
    # ============================================================================
    # Check 3: Tests
    # ============================================================================
    if (-not $SkipTests) {
        Write-Step "Check 6: Tests"
        try {
            cargo test --all-features --workspace 2>&1 | Out-Host
            if ($LASTEXITCODE -ne 0) { throw "Tests failed" }
            Write-Success "Tests OK"
        } catch {
            Write-Fail "Tests failed!"
            $errors += "Tests"
        }
    } else {
        Write-Warn "Skipping tests (-SkipTests)"
    }

    # ============================================================================
    # Check 4: Security Audit
    # ============================================================================
    if (-not $SkipSecurity) {
        Write-Step "Check 7: Security Audit (cargo deny)"
        try {
            cargo deny check 2>&1 | Out-Host
            if ($LASTEXITCODE -ne 0) { throw "Security audit failed" }
            Write-Success "Security audit OK"
        } catch {
            Write-Warn "Security audit found issues (non-blocking)"
        }
    } else {
        Write-Warn "Skipping security audit (-SkipSecurity)"
    }

    # ============================================================================
    # Check 5: Miri (optional, EPIC-032/US-004)
    # ============================================================================
    if ($Miri) {
        Write-Step "Check 8: Miri - Undefined Behavior Detection"
        try {
            # Check if nightly is available
            $nightlyCheck = rustup run nightly rustc --version 2>&1
            if ($LASTEXITCODE -eq 0) {
                # Install miri if not present
                rustup run nightly cargo miri --version 2>&1 | Out-Null
                if ($LASTEXITCODE -ne 0) {
                    Write-Host "   Installing Miri..." -ForegroundColor Yellow
                    rustup +nightly component add miri 2>&1 | Out-Host
                }
                # Run miri on core crate (limited scope for speed)
                Write-Host "   Running Miri on velesdb-core (unsafe code audit)..." -ForegroundColor Cyan
                Push-Location crates/velesdb-core
                try {
                    cargo +nightly miri test --lib -- --test-threads=1 2>&1 | Out-Host
                    if ($LASTEXITCODE -ne 0) { throw "Miri found UB" }
                    Write-Success "Miri OK - No undefined behavior detected"
                } finally {
                    Pop-Location
                }
            } else {
                Write-Warn "Nightly toolchain not installed - skipping Miri"
                Write-Host "   Install with: rustup toolchain install nightly" -ForegroundColor Yellow
            }
        } catch {
            Write-Fail "Miri detected undefined behavior!"
            $errors += "Miri"
        }
    } else {
        Write-Warn "Skipping Miri (use -Miri flag to enable)"
    }

    # ============================================================================
    # Check 6: File size validation
    # ============================================================================
    Write-Step "Check: File size validation (< 500 lines)"
    $largeFiles = Get-ChildItem -Path "crates" -Recurse -Filter "*.rs" | 
        Where-Object { (Get-Content $_.FullName | Measure-Object -Line).Lines -gt 500 } |
        Select-Object FullName, @{N='Lines';E={(Get-Content $_.FullName | Measure-Object -Line).Lines}}
    
    if ($largeFiles) {
        Write-Warn "Files exceeding 500 lines:"
        $largeFiles | ForEach-Object { Write-Host "   - $($_.FullName): $($_.Lines) lines" -ForegroundColor Yellow }
    } else {
        Write-Success "All files under 500 lines"
    }
}

# ============================================================================
# Summary
# ============================================================================
$duration = (Get-Date) - $startTime
Write-Host "`n═══════════════════════════════════════════════════════════════════" -ForegroundColor Blue

if ($errors.Count -eq 0) {
    Write-Host "  🎉 LOCAL CI PASSED - Ready to push!" -ForegroundColor Green
    Write-Host "  Duration: $($duration.TotalSeconds.ToString('F1'))s" -ForegroundColor Blue
    Write-Host "═══════════════════════════════════════════════════════════════════" -ForegroundColor Blue
    Write-Host "`n  Next steps:" -ForegroundColor Cyan
    Write-Host "    git push origin <branch>" -ForegroundColor White
    Write-Host ""
    exit 0
} else {
    Write-Host "  ❌ LOCAL CI FAILED - Fix issues before pushing!" -ForegroundColor Red
    Write-Host "  Failed checks: $($errors -join ', ')" -ForegroundColor Red
    Write-Host "  Duration: $($duration.TotalSeconds.ToString('F1'))s" -ForegroundColor Blue
    Write-Host "═══════════════════════════════════════════════════════════════════" -ForegroundColor Blue
    exit 1
}
