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
Write-Step "Check 1/5: Formatting (rustfmt)"
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
Write-Step "Check 2/5: Linting (clippy)"
try {
    cargo clippy --all-targets --all-features -- -D warnings -D clippy::pedantic 2>&1 | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "Clippy failed" }
    Write-Success "Clippy OK"
} catch {
    Write-Fail "Clippy found issues!"
    $errors += "Clippy"
}

if ($Quick) {
    Write-Warn "Quick mode - skipping tests and security audit"
} else {
    # ============================================================================
    # Check 3: Tests
    # ============================================================================
    if (-not $SkipTests) {
        Write-Step "Check 3/5: Tests"
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
        Write-Step "Check 4/5: Security Audit (cargo deny)"
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
        Write-Step "Check 5/6: Miri - Undefined Behavior Detection"
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
    Write-Step "Check 6/6: File size validation (< 500 lines)"
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
