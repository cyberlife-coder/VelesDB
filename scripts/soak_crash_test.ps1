# =============================================================================
# VelesDB Soak Crash Recovery Test Script
# =============================================================================
# Runs multiple crash recovery iterations to find edge cases.
#
# Usage:
#   .\scripts\soak_crash_test.ps1 -Iterations 100
#
# Parameters:
#   -Iterations    Number of test iterations (default: 100)
#   -Count         Vectors per iteration (default: 5000)
#   -SkipBuild     Skip initial build
# =============================================================================

param(
    [int]$Iterations = 100,
    [int]$Count = 5000,
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

Write-Host "=== VelesDB Soak Crash Recovery Test ===" -ForegroundColor Cyan
Write-Host "Iterations: $Iterations"
Write-Host "Vectors per iteration: $Count"
Write-Host ""

# Build once at the start
if (-not $SkipBuild) {
    Write-Host "Building crash driver..." -ForegroundColor Yellow
    cargo build --release --example crash_driver 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Write-Host "ERROR: Failed to build crash driver" -ForegroundColor Red
        exit 1
    }
    Write-Host "Build complete" -ForegroundColor Green
    Write-Host ""
}

$startTime = Get-Date
$failedSeeds = @()

for ($i = 1; $i -le $Iterations; $i++) {
    $seed = Get-Random -Minimum 1 -Maximum 999999
    $crashMs = Get-Random -Minimum 1000 -Maximum 10000
    
    Write-Host "=== Iteration $i/$Iterations (seed: $seed, crash: ${crashMs}ms) ===" -ForegroundColor Cyan
    
    & .\scripts\crash_test.ps1 -Seed $seed -Count $Count -CrashAfterMs $crashMs -SkipBuild
    
    if ($LASTEXITCODE -ne 0) {
        Write-Host "FAILED at iteration $i with seed $seed" -ForegroundColor Red
        $failedSeeds += $seed
        
        # Continue testing to find more failures
        Write-Host "Continuing to find more failures..." -ForegroundColor Yellow
    }
    
    Write-Host ""
}

$endTime = Get-Date
$duration = $endTime - $startTime

Write-Host ""
Write-Host "=== SOAK TEST SUMMARY ===" -ForegroundColor Cyan
Write-Host "Total iterations: $Iterations"
Write-Host "Duration: $($duration.TotalMinutes.ToString('F1')) minutes"
Write-Host "Failed: $($failedSeeds.Count)"

if ($failedSeeds.Count -gt 0) {
    Write-Host ""
    Write-Host "Failed seeds (for reproduction):" -ForegroundColor Red
    foreach ($seed in $failedSeeds) {
        Write-Host "  .\scripts\crash_test.ps1 -Seed $seed -Count $Count"
    }
    Write-Host ""
    Write-Host "SOAK TEST FAILED" -ForegroundColor Red
    exit 1
} else {
    Write-Host ""
    Write-Host "ALL $Iterations ITERATIONS PASSED!" -ForegroundColor Green
    exit 0
}
