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
    @{
        Path = "sdks/typescript/package.json"
        Pattern = '"@wiscale/velesdb-wasm": "\^\d+\.\d+\.\d+"'
        Replacement = "`"@wiscale/velesdb-wasm`": `"^$Version`""
        Description = "TypeScript SDK WASM dep"
    },
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
    # Inter-crate dependencies (velesdb-core version in other crates)
    @{
        Path = "crates/velesdb-server/Cargo.toml"
        Pattern = 'velesdb-core = \{ path = "\.\./velesdb-core", version = "\d+\.\d+\.\d+" \}'
        Replacement = "velesdb-core = { path = `"../velesdb-core`", version = `"$Version`" }"
        Description = "velesdb-server -> core dep"
    },
    @{
        Path = "crates/velesdb-python/Cargo.toml"
        Pattern = 'velesdb-core = \{ path = "\.\./velesdb-core", version = "\d+\.\d+\.\d+" \}'
        Replacement = "velesdb-core = { path = `"../velesdb-core`", version = `"$Version`" }"
        Description = "velesdb-python -> core dep"
    },
    @{
        Path = "crates/velesdb-cli/Cargo.toml"
        Pattern = 'velesdb-core = \{ path = "\.\./velesdb-core", version = "\d+\.\d+\.\d+" \}'
        Replacement = "velesdb-core = { path = `"../velesdb-core`", version = `"$Version`" }"
        Description = "velesdb-cli -> core dep"
    },
    @{
        Path = "crates/velesdb-migrate/Cargo.toml"
        Pattern = 'velesdb-core = \{ version = "\d+\.\d+\.\d+", path = "\.\./velesdb-core" \}'
        Replacement = "velesdb-core = { version = `"$Version`", path = `"../velesdb-core`" }"
        Description = "velesdb-migrate -> core dep"
    },
    @{
        Path = "crates/velesdb-mobile/Cargo.toml"
        Pattern = 'velesdb-core = \{ path = "\.\./velesdb-core", version = "\d+\.\d+\.\d+" \}'
        Replacement = "velesdb-core = { path = `"../velesdb-core`", version = `"$Version`" }"
        Description = "velesdb-mobile -> core dep"
    },
    @{
        Path = "crates/tauri-plugin-velesdb/Cargo.toml"
        Pattern = 'velesdb-core = \{ path = "\.\./\.\./crates/velesdb-core", version = "\d+\.\d+\.\d+" \}'
        Replacement = "velesdb-core = { path = `"../../crates/velesdb-core`", version = `"$Version`" }"
        Description = "tauri-plugin -> core dep"
    }
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
