# =============================================================================
# VelesDB - Publish Python Bindings to PyPI
# =============================================================================
# Usage: .\scripts\publish-pypi.ps1
# Requires: .env file with MATURIN_PYPI_TOKEN set
# =============================================================================

$ErrorActionPreference = "Stop"

# Load .env file from project root
$envFile = Join-Path $PSScriptRoot ".." ".env"
if (Test-Path $envFile) {
    Write-Host "Loading .env file..." -ForegroundColor Cyan
    Get-Content $envFile | ForEach-Object {
        if ($_ -match "^\s*([^#][^=]*)\s*=\s*(.*)\s*$") {
            $name = $matches[1].Trim()
            $value = $matches[2].Trim()
            if ($value) {
                [Environment]::SetEnvironmentVariable($name, $value, "Process")
                Write-Host "  Loaded: $name" -ForegroundColor DarkGray
            }
        }
    }
} else {
    Write-Host "No .env file found at $envFile" -ForegroundColor Yellow
    Write-Host "Create one from .env.example with your MATURIN_PYPI_TOKEN" -ForegroundColor Yellow
    exit 1
}

# Check token
if (-not $env:MATURIN_PYPI_TOKEN) {
    Write-Host "Error: MATURIN_PYPI_TOKEN not set in .env" -ForegroundColor Red
    Write-Host "Get your token from: https://pypi.org/manage/account/token/" -ForegroundColor Yellow
    exit 1
}

Write-Host ""
Write-Host "Publishing velesdb to PyPI..." -ForegroundColor Green
Write-Host "================================" -ForegroundColor Green

# Navigate to velesdb-python crate
$pythonCrate = Join-Path $PSScriptRoot ".." "crates" "velesdb-python"
Push-Location $pythonCrate

try {
    # Run maturin publish
    maturin publish
    
    if ($LASTEXITCODE -eq 0) {
        Write-Host ""
        Write-Host "Successfully published to PyPI!" -ForegroundColor Green
        Write-Host "View at: https://pypi.org/project/velesdb/" -ForegroundColor Cyan
    } else {
        Write-Host "Publication failed with exit code $LASTEXITCODE" -ForegroundColor Red
        exit $LASTEXITCODE
    }
} finally {
    Pop-Location
}
