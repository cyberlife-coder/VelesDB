#!/usr/bin/env pwsh
# VelesDB Pre-Write Hook - Safety Checks Before Code Modification
# Can BLOCK the write by exiting with code 2

param()

$ErrorActionPreference = "Continue"

# Read JSON from stdin
$jsonInput = $input | Out-String
$data = $jsonInput | ConvertFrom-Json -ErrorAction SilentlyContinue

if (-not $data) {
    exit 0
}

$filePath = $data.tool_info.file_path
$edits = $data.tool_info.edits

# Only check Rust files
if ($filePath -notmatch '\.rs$') {
    exit 0
}

$blockWrite = $false
$warnings = @()

foreach ($edit in $edits) {
    $newCode = $edit.new_string
    
    # BLOCK: unsafe without SAFETY comment in new code
    if ($newCode -match 'unsafe\s*\{' -and $newCode -notmatch '// SAFETY:') {
        $warnings += "🔴 BLOCKED: unsafe block requires // SAFETY: comment explaining why it's safe"
        $blockWrite = $true
    }
    
    # BLOCK: Direct SQL string concatenation (security risk)
    if ($newCode -match 'format!\s*\([^)]*SELECT|INSERT|UPDATE|DELETE' -and $newCode -notmatch 'sanitize') {
        $warnings += "🔴 BLOCKED: Potential SQL injection - use parameterized queries"
        $blockWrite = $true
    }
}

# WARN: Modifying critical API files
$criticalFiles = @('lib.rs', 'mod.rs', 'error.rs')
$fileName = [System.IO.Path]::GetFileName($filePath)

if ($criticalFiles -contains $fileName) {
    Write-Host "⚠️  WARNING: Modifying critical API file: $fileName" -ForegroundColor Yellow
    Write-Host "   Consider running /impact-analysis first" -ForegroundColor Yellow
}

# WARN: Modifying public API (pub fn, pub struct, pub enum)
foreach ($edit in $edits) {
    if ($edit.old_string -match 'pub (fn|struct|enum|trait)' -and $edit.new_string -match 'pub (fn|struct|enum|trait)') {
        $oldSignature = ($edit.old_string -split '\n')[0]
        $newSignature = ($edit.new_string -split '\n')[0]
        if ($oldSignature -ne $newSignature) {
            Write-Host "⚠️  PUBLIC API CHANGE detected!" -ForegroundColor Yellow
            Write-Host "   Old: $oldSignature" -ForegroundColor DarkGray
            Write-Host "   New: $newSignature" -ForegroundColor White
            Write-Host "   → Update SDK bindings if breaking change" -ForegroundColor Yellow
        }
    }
}

if ($blockWrite) {
    Write-Host "`n━━━ WRITE BLOCKED ━━━" -ForegroundColor Red
    foreach ($warning in $warnings) {
        Write-Host $warning -ForegroundColor Red
    }
    Write-Host "Fix these issues before proceeding.`n" -ForegroundColor Red
    exit 2  # Exit code 2 BLOCKS the action
}

exit 0
