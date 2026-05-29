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
    - Dockerfiles (LABEL version="...")

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
        Path = "integrations/haystack/pyproject.toml"
        Pattern = 'version = "\d+\.\d+\.\d+"'
        Replacement = "version = `"$Version`""
        Description = "Haystack integration"
    },
    @{
        Path = "integrations/haystack/src/haystack_velesdb/__init__.py"
        Pattern = '__version__ = "\d+\.\d+\.\d+"'
        Replacement = "__version__ = `"$Version`""
        Description = "Haystack __init__.py __version__"
    },
    # Doc files with hardcoded version banners that must track the workspace.
    # Discovered as drift in Devin review on PR #723: server README health JSON
    # and Python README badge stayed at 1.14.0 while workspace bumped to 1.14.1.
    @{
        Path = "crates/velesdb-server/README.md"
        Pattern = '"version": "\d+\.\d+\.\d+"'
        Replacement = "`"version`": `"$Version`""
        Description = "velesdb-server README health JSON"
    },
    @{
        Path = "crates/velesdb-python/README.md"
        Pattern = 'version-\d+\.\d+\.\d+-blue'
        Replacement = "version-$Version-blue"
        Description = "velesdb-python README version badge"
    },
    @{
        Path = "examples/wasm-browser-demo/index.html"
        Pattern = '@wiscale/velesdb-wasm@\d+\.\d+\.\d+/'
        Replacement = "@wiscale/velesdb-wasm@$Version/"
        Description = "wasm-browser-demo index.html CDN URLs"
    },
    @{
        Path = "docs/guides/CONFIGURATION.md"
        Pattern = '# Version: \d+\.\d+\.\d+'
        Replacement = "# Version: $Version"
        Description = "CONFIGURATION.md TOML example header"
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
    },
    # Doc snippets that mirror /health and /ready response bodies — the server
    # echoes the workspace version, so the example in the docs has to track it
    # to remain accurate. v1.13.0 -> v1.13.7 drift was caught manually before
    # v1.13.8 because no tooling policed it; this entry wires it in.
    @{
        Path = "docs/getting-started.md"
        Pattern = '"version":\s*"\d+\.\d+\.\d+"'
        Replacement = "`"version`": `"$Version`""
        Description = "getting-started.md /health snippet"
    },
    @{
        Path = "docs/reference/api-reference.md"
        Pattern = '"version":\s*"\d+\.\d+\.\d+"'
        Replacement = "`"version`": `"$Version`""
        Description = "api-reference.md /health snippet"
    },
    @{
        Path = "docs/guides/SERVER_SECURITY.md"
        Pattern = '"version":\s*"\d+\.\d+\.\d+"'
        Replacement = "`"version`": `"$Version`""
        Description = "SERVER_SECURITY.md /health and /ready snippets"
    },
    # Dockerfile LABEL version drift was undetectable until v1.13.7 — the root
    # Dockerfile shipped a stale `1.12.0` label across seven patch releases
    # (see docs/quickstart/timing-results.md honesty note #3). Each Dockerfile
    # is anchored on `^LABEL version=` so we never accidentally rewrite an
    # arbitrary version string elsewhere in the file.
    @{
        Path = "Dockerfile"
        Pattern = '(?m)^LABEL version="\d+\.\d+\.\d+"'
        Replacement = "LABEL version=`"$Version`""
        Description = "Dockerfile LABEL (root, build + runtime stages)"
    },
    @{
        Path = "benchmarks/Dockerfile.optimized"
        Pattern = '(?m)^LABEL version="\d+\.\d+\.\d+"'
        Replacement = "LABEL version=`"$Version`""
        Description = "benchmarks/Dockerfile.optimized LABEL"
    },
    @{
        Path = "benchmarks/Dockerfile.nightly"
        Pattern = '(?m)^LABEL version="\d+\.\d+\.\d+"'
        Replacement = "LABEL version=`"$Version`""
        Description = "benchmarks/Dockerfile.nightly LABEL"
    },
    @{
        Path = "benchmarks/Dockerfile.bench"
        Pattern = '(?m)^LABEL version="\d+\.\d+\.\d+"'
        Replacement = "LABEL version=`"$Version`""
        Description = "benchmarks/Dockerfile.bench LABEL"
    },
    # New entries added in v1.14.x -> v1.14.2 audit (2026-05-01) to police
    # banners and version pins that the previous tooling missed. Each was
    # found drifting silently across one or more releases and is now tracked.
    @{
        Path = "integrations/langchain/src/langchain_velesdb/__init__.py"
        Pattern = '__version__ = "\d+\.\d+\.\d+"'
        Replacement = "__version__ = `"$Version`""
        Description = "LangChain __init__.py __version__"
    },
    @{
        Path = "integrations/llamaindex/src/llamaindex_velesdb/__init__.py"
        Pattern = '__version__ = "\d+\.\d+\.\d+"'
        Replacement = "__version__ = `"$Version`""
        Description = "LlamaIndex __init__.py __version__"
    },
    @{
        Path = "docs/openapi.yaml"
        # Anchored on the 2-space indent unique to the .info.version key.
        Pattern = '(?m)^  version:\s*\d+\.\d+\.\d+'
        Replacement = "  version: $Version"
        Description = "OpenAPI YAML spec (.info.version)"
    },
    @{
        Path = "sdks/typescript/README.md"
        # Bold banner directly under the title: `**vX.Y.Z** | Node.js >= 18 | ...`
        Pattern = '(?m)^\*\*v\d+\.\d+\.\d+\*\*'
        Replacement = "**v$Version**"
        Description = "TS SDK README **vX.Y.Z** banner"
    },
    @{
        Path = "ROADMAP.md"
        Pattern = 'covers v\d+\.\d+\.\d+ \(current\)'
        Replacement = "covers v$Version (current)"
        Description = "ROADMAP.md `covers vX.Y.Z (current)` marker"
    },
    @{
        Path = "docs/guides/CLI_REPL.md"
        # Two occurrences in this guide: header banner + `velesdb X.Y.Z`
        # in the --version sample output + table cell. Replace-all is safe
        # because every X.Y.Z in CLI_REPL.md refers to the workspace version.
        Pattern = '\d+\.\d+\.\d+'
        Replacement = "$Version"
        Description = "docs/guides/CLI_REPL.md (banner + sample outputs)"
    },
    @{
        Path = "docs/guides/CONFIGURATION.md"
        # Markdown header (line 3). The TOML `# Version: X.Y.Z` inside the
        # code block is policed separately by the existing `doc_toml_header`
        # entry above — anchored differently to avoid clashes here.
        Pattern = '(?m)^\*Version \d+\.\d+\.\d+'
        Replacement = "*Version $Version"
        Description = "docs/guides/CONFIGURATION.md *Version banner"
    },
    @{
        Path = "docs/guides/GRAPH_PATTERNS.md"
        Pattern = '(?m)^\*Version \d+\.\d+\.\d+'
        Replacement = "*Version $Version"
        Description = "docs/guides/GRAPH_PATTERNS.md *Version banner"
    },
    @{
        Path = "docs/guides/SEARCH_MODES.md"
        Pattern = '(?m)^\*Version \d+\.\d+\.\d+'
        Replacement = "*Version $Version"
        Description = "docs/guides/SEARCH_MODES.md *Version banner"
    },
    @{
        Path = "docs/BENCHMARKS.md"
        # Anchored on `Last updated:` so we never accidentally rewrite
        # historical (vX.Y.Z) references elsewhere in the file.
        Pattern = '(Last updated:[^\n]*?)\(v\d+\.\d+\.\d+\)'
        Replacement = "`${1}(v$Version)"
        Description = "docs/BENCHMARKS.md Last updated stamp"
    },
    @{
        Path = "docs/reference/ECOSYSTEM_PARITY.md"
        # `Last updated: YYYY-MM-DD (vX.Y.Z - ...)` - anchored on
        # `Last updated:` to skip historical references in the body.
        Pattern = '(Last updated:[^\n]*?)\(v\d+\.\d+\.\d+'
        Replacement = "`${1}(v$Version"
        Description = "docs/reference/ECOSYSTEM_PARITY.md last-updated stamp"
    },
    @{
        Path = "docs/reference/VELESQL_CONFORMANCE_MATRIX.md"
        # `(v3.9.0 / VelesDB v1.14.2)` - only the trailing VelesDB version
        # tracks the workspace. Anchored on `Last updated:` to leave
        # historical "VelesDB v1.13.0 (PR #629)" body references intact.
        Pattern = '(Last updated:[^\n]*?VelesDB v)\d+\.\d+\.\d+'
        Replacement = "`${1}$Version"
        Description = "docs/reference/VELESQL_CONFORMANCE_MATRIX.md last-updated stamp"
    },
    @{
        Path = "docs/reference/ARCHITECTURE_DIAGRAMS.md"
        # First-line h1 `# VelesDB Architecture Diagrams — vX.Y.Z`
        Pattern = '— v\d+\.\d+\.\d+'
        Replacement = "— v$Version"
        Description = "docs/reference/ARCHITECTURE_DIAGRAMS.md h1 title"
    },
    @{
        Path = "scripts/dx-timing/scenario_rust.sh"
        Pattern = 'velesdb-core@\d+\.\d+\.\d+'
        Replacement = "velesdb-core@$Version"
        Description = "scripts/dx-timing/scenario_rust.sh cargo pin"
    },
    @{
        Path = "scripts/dx-timing/scenario_server.sh"
        Pattern = 'velesdb-server@\d+\.\d+\.\d+'
        Replacement = "velesdb-server@$Version"
        Description = "scripts/dx-timing/scenario_server.sh cargo pin"
    },
    # Install guide pins the pre-built multi-arch GHCR image (added v1.16.0).
    # Only the `:X.Y.Z` tag is rewritten; the adjacent `:latest` example is
    # left untouched (the \d+\.\d+\.\d+ pattern never matches `latest`).
    @{
        Path = "docs/guides/INSTALLATION.md"
        Pattern = 'ghcr\.io/cyberlife-coder/velesdb:\d+\.\d+\.\d+'
        Replacement = "ghcr.io/cyberlife-coder/velesdb:$Version"
        Description = "docs/guides/INSTALLATION.md GHCR image pin"
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
