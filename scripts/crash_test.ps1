# =============================================================================
# VelesDB Crash Recovery Test Script
# =============================================================================
# Simulates crash scenarios and validates recovery.
#
# Usage:
#   .\scripts\crash_test.ps1 -Seed 42 -Count 10000 -CrashAfterMs 5000
#
# Parameters:
#   -Seed          Random seed for reproducibility (default: random)
#   -Count         Number of vectors to insert (default: 10000)
#   -CrashAfterMs  Kill process after this many milliseconds (default: 5000)
#   -Dimension     Vector dimension (default: 128)
#   -SkipBuild     Skip cargo build step
# =============================================================================

param(
    [int]$Seed = (Get-Random -Minimum 1 -Maximum 999999),
    [int]$Count = 10000,
    [int]$CrashAfterMs = 5000,
    [int]$Dimension = 128,
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

# Configuration
$TestDir = "./target/crash_test_$Seed"

Write-Host "=== VelesDB Crash Recovery Test ===" -ForegroundColor Cyan
Write-Host "Seed: $Seed"
Write-Host "Count: $Count"
Write-Host "Crash after: $CrashAfterMs ms"
Write-Host "Dimension: $Dimension"
Write-Host "Test dir: $TestDir"
Write-Host ""

# Step 1: Build the driver (if not skipping)
if (-not $SkipBuild) {
    Write-Host "[1/5] Building crash driver..." -ForegroundColor Yellow
    cargo build --release --example crash_driver 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Write-Host "ERROR: Failed to build crash driver" -ForegroundColor Red
        exit 1
    }
    Write-Host "      Build complete" -ForegroundColor Green
} else {
    Write-Host "[1/5] Skipping build (--SkipBuild)" -ForegroundColor Yellow
}

# Step 2: Clean previous test data
Write-Host "[2/5] Cleaning previous test data..." -ForegroundColor Yellow
if (Test-Path $TestDir) {
    Remove-Item -Recurse -Force $TestDir
}
New-Item -ItemType Directory -Path $TestDir -Force | Out-Null
Write-Host "      Clean complete" -ForegroundColor Green

# Step 3: Start driver in background and crash it
Write-Host "[3/5] Starting insert operation..." -ForegroundColor Yellow

$processArgs = @(
    "run", "--release", "--example", "crash_driver", "--",
    "--mode", "insert",
    "--seed", $Seed,
    "--count", $Count,
    "--dimension", $Dimension,
    "--data-dir", $TestDir
)

$process = Start-Process -FilePath "cargo" -ArgumentList $processArgs -PassThru -NoNewWindow

# Wait random time then kill (simulating crash)
$actualWait = Get-Random -Minimum ([int]($CrashAfterMs * 0.5)) -Maximum ([int]($CrashAfterMs * 1.5))
Write-Host "      Waiting $actualWait ms before crash..." -ForegroundColor Gray

Start-Sleep -Milliseconds $actualWait

# Check if process is still running
if (-not $process.HasExited) {
    Write-Host "      Killing process (simulating crash)..." -ForegroundColor Yellow
    Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
    Write-Host "      Process killed after $actualWait ms" -ForegroundColor Green
} else {
    Write-Host "      Process completed before crash timeout" -ForegroundColor Yellow
}

# Small delay to ensure files are released
Start-Sleep -Milliseconds 500

# Step 4: Run integrity check (recovery)
Write-Host "[4/5] Running integrity check (recovery)..." -ForegroundColor Yellow

$checkArgs = @(
    "run", "--release", "--example", "crash_driver", "--",
    "--mode", "check",
    "--seed", $Seed,
    "--dimension", $Dimension,
    "--data-dir", $TestDir
)

$checkProcess = Start-Process -FilePath "cargo" -ArgumentList $checkArgs -PassThru -NoNewWindow -Wait

if ($checkProcess.ExitCode -eq 0) {
    Write-Host "      Integrity check PASSED" -ForegroundColor Green
} else {
    Write-Host "      Integrity check FAILED (exit code: $($checkProcess.ExitCode))" -ForegroundColor Red
    Write-Host ""
    Write-Host "=== REPRODUCTION INFO ===" -ForegroundColor Yellow
    Write-Host "Seed: $Seed"
    Write-Host "Command: .\scripts\crash_test.ps1 -Seed $Seed -Count $Count -CrashAfterMs $CrashAfterMs"
    Write-Host "=========================" -ForegroundColor Yellow
    exit 1
}

# Step 5: Summary
Write-Host "[5/5] Test complete" -ForegroundColor Yellow
Write-Host ""
Write-Host "=== CRASH RECOVERY TEST PASSED ===" -ForegroundColor Green
Write-Host "Seed: $Seed"
Write-Host "Crash after: $actualWait ms"
Write-Host ""

# Cleanup (optional)
# Remove-Item -Recurse -Force $TestDir

exit 0
