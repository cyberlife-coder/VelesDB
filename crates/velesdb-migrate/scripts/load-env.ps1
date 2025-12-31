# Load environment variables from .env file for migration testing
# Usage: . .\crates\velesdb-migrate\scripts\load-env.ps1

param(
    [string]$EnvFile = ".env"
)

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$projectRoot = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $scriptDir))

# Try to find .env file
$envPath = if (Test-Path $EnvFile) { 
    $EnvFile 
} elseif (Test-Path (Join-Path $projectRoot ".env")) {
    Join-Path $projectRoot ".env"
} else {
    Write-Host "❌ No .env file found" -ForegroundColor Red
    Write-Host "   Create one from .env.example:" -ForegroundColor Yellow
    Write-Host "   Copy-Item .env.example .env" -ForegroundColor Gray
    return
}

Write-Host "📁 Loading environment from: $envPath" -ForegroundColor Cyan

# Parse and set environment variables
Get-Content $envPath | ForEach-Object {
    if ($_ -match '^\s*([^#][^=]+)=(.*)$') {
        $name = $matches[1].Trim()
        $value = $matches[2].Trim()
        if ($value -and $value -ne "your-service-role-key" -and $value -notlike "*YOUR_PROJECT*") {
            Set-Item -Path "env:$name" -Value $value
            Write-Host "   ✅ $name" -ForegroundColor Green
        }
    }
}

Write-Host ""
Write-Host "🚀 Ready to test! Run:" -ForegroundColor Cyan
Write-Host "   cargo test -p velesdb-migrate --test integration_test -- --ignored --nocapture" -ForegroundColor Gray
Write-Host "   cargo bench -p velesdb-migrate" -ForegroundColor Gray
