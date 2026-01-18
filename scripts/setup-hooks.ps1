# =============================================================================
# VelesDB - Git Hooks Setup Script
# =============================================================================
# Configure git to use the project's custom hooks (pre-commit, pre-push)
# for local CI validation before pushing to origin.
#
# Usage: .\scripts\setup-hooks.ps1
# =============================================================================

$ErrorActionPreference = "Stop"

Write-Host "`n" -NoNewline
Write-Host "═══════════════════════════════════════════════════════════════════" -ForegroundColor Blue
Write-Host "  VelesDB - Git Hooks Setup" -ForegroundColor Blue
Write-Host "═══════════════════════════════════════════════════════════════════" -ForegroundColor Blue

# Configure git to use .githooks directory
Write-Host "`n📋 Configuring git hooks path..." -ForegroundColor Cyan
git config core.hooksPath .githooks

# Verify configuration
$hooksPath = git config --get core.hooksPath
if ($hooksPath -eq ".githooks") {
    Write-Host "✅ Git hooks configured successfully!" -ForegroundColor Green
    Write-Host "   Hooks directory: .githooks/" -ForegroundColor White
    Write-Host ""
    Write-Host "📌 Active hooks:" -ForegroundColor Cyan
    
    if (Test-Path ".githooks/pre-commit") {
        Write-Host "   - pre-commit  : Validates code before each commit" -ForegroundColor White
    }
    if (Test-Path ".githooks/pre-push") {
        Write-Host "   - pre-push    : Full CI validation before push to origin" -ForegroundColor White
    }
    
    Write-Host ""
    Write-Host "💡 Workflow:" -ForegroundColor Yellow
    Write-Host "   1. Make your changes" -ForegroundColor White
    Write-Host "   2. git commit -m '...'     # pre-commit hook runs" -ForegroundColor White
    Write-Host "   3. .\scripts\local-ci.ps1  # Optional: full validation" -ForegroundColor White
    Write-Host "   4. git push origin <branch> # pre-push hook runs" -ForegroundColor White
    Write-Host ""
} else {
    Write-Host "❌ Failed to configure git hooks" -ForegroundColor Red
    exit 1
}

Write-Host "═══════════════════════════════════════════════════════════════════" -ForegroundColor Blue
Write-Host ""
