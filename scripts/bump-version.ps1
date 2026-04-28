<#
.SYNOPSIS
    Bump version across all VelesDB components.

.DESCRIPTION
    Updates version numbers in all package files to ensure SemVer consistency:
    - Cargo.toml (workspace)
    - TypeScript SDK (package.json)
    - Python SDK (pyproject.toml)
    - WASM package (pkg/package.json)
    - Tauri plugin (guest-js/package.json)
    - LangChain integration (pyproject.toml)
    - LlamaIndex integration (pyproject.toml)
    - RAG demo (pyproject.toml)

.PARAMETER Version
    The new version number (e.g., "0.8.9")

.PARAMETER DryRun
    Show what would be changed without modifying files

.EXAMPLE
    .\bump-version.ps1 -Version "0.9.0"
    .\bump-version.ps1 -Version "0.9.0" -DryRun
#>

param(
    [Parameter(Mandatory=$true)]
    [ValidatePattern('^\d+\.\d+\.\d+$')]
    [string]$Version,
    
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RootDir = Split-Path -Parent $ScriptDir

Write-Host "🔄 VelesDB Version Bump to v$Version" -ForegroundColor Cyan
if ($DryRun) {
    Write-Host "   (DRY RUN - no files will be modified)" -ForegroundColor Yellow
}
Write-Host ""

# Files to update with their patterns
$FilesToUpdate = @(
    @{
        Path = "Cargo.toml"
        Pattern = 'version = "\d+\.\d+\.\d+"'
        Replacement = "version = `"$Version`""
        Description = "Cargo workspace"
    },
    @{
        Path = "sdks/typescript/package.json"
        Pattern = '"version": "\d+\.\d+\.\d+"'
        Replacement = "`"version`": `"$Version`""
        Description = "TypeScript SDK"
    },
    # NOTE: @wiscale/velesdb-wasm dep in sdks/typescript/package.json is intentionally
    # NOT auto-bumped here. The WASM package follows its own versioning track (currently
    # ^1.4.1 stable). Bumping it to the workspace version would target an unpublished
    # version on npm, breaking 'npm ci' (chicken-and-egg). When you genuinely want to
    # advance the WASM dep, edit sdks/typescript/package.json manually and regenerate
    # the lockfile. (Devin finding #705-B, 2026-04-28.)
    @{
        Path = "crates/velesdb-python/pyproject.toml"
        Pattern = 'version = "\d+\.\d+\.\d+"'
        Replacement = "version = `"$Version`""
        Description = "Python SDK"
    },
    @{
        Path = "crates/velesdb-wasm/pkg/package.json"
        Pattern = '"version": "\d+\.\d+\.\d+"'
        Replacement = "`"version`": `"$Version`""
        Description = "WASM package"
    },
    @{
        Path = "crates/tauri-plugin-velesdb/guest-js/package.json"
        Pattern = '"version": "\d+\.\d+\.\d+"'
        Replacement = "`"version`": `"$Version`""
        Description = "Tauri plugin JS"
    },
    @{
        Path = "integrations/common/pyproject.toml"
        Pattern = 'version = "\d+\.\d+\.\d+"'
        Replacement = "version = `"$Version`""
        Description = "VelesDB common integration helpers"
    },
    @{
        Path = "integrations/langchain/pyproject.toml"
        Pattern = 'version = "\d+\.\d+\.\d+"'
        Replacement = "version = `"$Version`""
        Description = "LangChain integration"
    },
    @{
        Path = "integrations/llamaindex/pyproject.toml"
        Pattern = 'version = "\d+\.\d+\.\d+"'
        Replacement = "version = `"$Version`""
        Description = "LlamaIndex integration"
    },
    @{
        Path = "demos/rag-pdf-demo/pyproject.toml"
        Pattern = 'version = "\d+\.\d+\.\d+"'
        Replacement = "version = `"$Version`""
        Description = "RAG demo"
    },
    @{
        Path = "docs/openapi.json"
        # Match the "version" field inside the .info object. Anchored on the
        # 4-space indent unique to the .info section in our spec to avoid hitting
        # any other "version" key elsewhere in the file.
        Pattern = '    "version": "\d+\.\d+\.\d+"'
        Replacement = "    `"version`": `"$Version`""
        Description = "OpenAPI spec (.info.version)"
    }
    # NOTE: per-crate inter-crate dependency entries (velesdb-server -> core,
    # velesdb-cli -> core, etc.) used to live here. They were removed in v1.13.6
    # because every workspace member now uses `velesdb-core = { workspace = true }`,
    # so the path/version pattern they targeted no longer matches anywhere — they
    # only inflated $ErrorCount and forced the script to exit 1. The single
    # workspace-level `velesdb-core = { path = ..., version = "..." }` line in the
    # root Cargo.toml is already covered by the "Cargo workspace" entry above
    # (the global -replace catches both the workspace.package version and the
    # workspace.dependencies version line). (Devin finding #705-D, 2026-04-28.)
)

$UpdatedCount = 0
$ErrorCount = 0

foreach ($file in $FilesToUpdate) {
    $FullPath = Join-Path $RootDir $file.Path
    
    if (-not (Test-Path $FullPath)) {
        Write-Host "⚠️  $($file.Description): File not found - $($file.Path)" -ForegroundColor Yellow
        continue
    }
    
    $Content = Get-Content $FullPath -Raw
    $OldVersion = [regex]::Match($Content, $file.Pattern).Value
    
    if ($OldVersion) {
        $NewContent = $Content -replace $file.Pattern, $file.Replacement
        
        if ($Content -ne $NewContent) {
            if (-not $DryRun) {
                Set-Content -Path $FullPath -Value $NewContent -NoNewline
            }
            Write-Host "✅ $($file.Description): $OldVersion → $($file.Replacement)" -ForegroundColor Green
            $UpdatedCount++
        } else {
            Write-Host "✓  $($file.Description): Already at $Version" -ForegroundColor DarkGray
        }
    } else {
        Write-Host "❌ $($file.Description): Pattern not found in $($file.Path)" -ForegroundColor Red
        $ErrorCount++
    }
}

Write-Host ""
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" -ForegroundColor Cyan

if ($DryRun) {
    Write-Host "DRY RUN complete. $UpdatedCount file(s) would be updated." -ForegroundColor Yellow
} else {
    Write-Host "✅ Version bump complete! $UpdatedCount file(s) updated." -ForegroundColor Green
    
    if ($ErrorCount -eq 0) {
        Write-Host ""
        Write-Host "Next steps:" -ForegroundColor Cyan
        Write-Host "  1. Review changes: git diff"
        Write-Host "  2. Commit: git add -A && git commit -m `"chore(release): bump version to $Version`""
        Write-Host "  3. Tag: git tag -a v$Version -m `"v$Version`""
        Write-Host "  4. Push: git push origin main --tags"
    }
}

if ($ErrorCount -gt 0) {
    Write-Host "⚠️  $ErrorCount error(s) occurred" -ForegroundColor Red
    exit 1
}
