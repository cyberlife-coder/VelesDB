#!/usr/bin/env pwsh
# VelesDB Post-Write Hook - Rust Quality Checks
# Runs after Cascade modifies a Rust file

param()

$ErrorActionPreference = "Continue"

# Read JSON from stdin
$jsonInput = $input | Out-String
$data = $jsonInput | ConvertFrom-Json -ErrorAction SilentlyContinue

if (-not $data) {
    Write-Host "No input data received" -ForegroundColor Yellow
    exit 0
}

$filePath = $data.tool_info.file_path
$edits = $data.tool_info.edits

# Only process Rust files
if ($filePath -notmatch '\.rs$') {
    exit 0
}

Write-Host "`n━━━ VelesDB Post-Write Hook ━━━" -ForegroundColor Cyan
Write-Host "📝 File: $filePath" -ForegroundColor White

# 1. Format the file
Write-Host "`n🔧 Running cargo fmt..." -ForegroundColor Yellow
cargo fmt --all -- --quiet 2>&1 | Out-Null

# 2. Run clippy on the specific file (fast check)
Write-Host "🔍 Running clippy check..." -ForegroundColor Yellow
$clippyOutput = cargo clippy --message-format=short -- -D warnings 2>&1
$clippyErrors = $clippyOutput | Select-String -Pattern "error\[" -SimpleMatch
if ($clippyErrors) {
    Write-Host "⚠️  Clippy warnings detected!" -ForegroundColor Red
    $clippyErrors | ForEach-Object { Write-Host $_ -ForegroundColor Red }
} else {
    Write-Host "✅ Clippy: OK" -ForegroundColor Green
}

# 3. Check for anti-patterns in the edits
$hasIssues = $false
foreach ($edit in $edits) {
    $newCode = $edit.new_string
    
    # Check for unwrap() without justification in hot paths
    if ($filePath -match '(simd|hnsw|storage|index)' -and $newCode -match '\.unwrap\(\)') {
        Write-Host "⚠️  unwrap() detected in hot-path file - consider using ? or expect()" -ForegroundColor Yellow
        $hasIssues = $true
    }
    
    # Check for clone() without justification in hot paths
    if ($filePath -match '(simd|hnsw|storage|index)' -and $newCode -match '\.clone\(\)' -and $newCode -notmatch '// clone:') {
        Write-Host "⚠️  clone() in hot-path - consider adding justification comment" -ForegroundColor Yellow
        $hasIssues = $true
    }
    
    # Check for unsafe without SAFETY comment
    if ($newCode -match 'unsafe\s*\{' -and $newCode -notmatch '// SAFETY:') {
        Write-Host "🔴 unsafe block without // SAFETY: comment!" -ForegroundColor Red
        $hasIssues = $true
    }
}

# 4. Suggest running tests for the modified module
$moduleName = [System.IO.Path]::GetFileNameWithoutExtension($filePath)
if ($moduleName -ne "mod" -and $moduleName -ne "lib") {
    Write-Host "`n💡 Tip: Run tests for this module:" -ForegroundColor Cyan
    Write-Host "   cargo test $moduleName" -ForegroundColor White
}

if ($hasIssues) {
    Write-Host "`n⚡ Quality issues detected - review before commit" -ForegroundColor Yellow
}

Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━`n" -ForegroundColor Cyan
exit 0
