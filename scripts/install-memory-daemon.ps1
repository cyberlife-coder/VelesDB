# =============================================================================
# VelesDB Memory Daemon Installer — Windows
# =============================================================================
# PowerShell mirror of scripts/install-memory-daemon.sh (read that script's
# header first — same rationale, same daemon, same client wiring). This file
# only documents where Windows *diverges* from the macOS original:
#
#   - No launchd on Windows, and a Windows *service* needs an admin install.
#     Instead this script registers a per-user **Scheduled Task** (folder
#     `\VelesDB\`, trigger "at logon") — `Register-ScheduledJob` is
#     deprecated, so this uses `Register-ScheduledTask`. A Scheduled Task
#     action carries no environment block, so the daemon's env vars
#     (VELESDB_MEMORY_PATH, etc.) are baked into a tiny generated wrapper
#     `run-daemon.cmd` that `set`s them and then execs the binary — the
#     task's Action just launches that wrapper.
#   - CA trust targets the **CurrentUser\Root** certificate store (no admin
#     needed), via `certutil -addstore -user Root` — chosen over
#     `Import-Certificate` because certutil accepts the daemon's PEM output
#     natively across PowerShell versions, matching the "prefer certutil for
#     compat" guidance this script follows. Idempotency is checked by
#     thumbprint before importing, same spirit as the macOS strict-curl
#     ground-truth check.
#   - Client config paths follow Windows conventions: Claude Desktop
#     `%APPDATA%\Claude\claude_desktop_config.json` (still stdio-only — never
#     written here, same as macOS), Windsurf
#     `%USERPROFILE%\.codeium\windsurf\mcp_config.json`, Devin CLI
#     `%APPDATA%\devin\config.json` (documented at
#     https://cli.devin.ai/docs/reference/configuration/config-file — Devin's
#     own docs give the Windows path explicitly, unlike most of this
#     ecosystem, so no guessing needed).
#   - JSON is edited with ConvertFrom-Json/ConvertTo-Json (no jq dependency
#     on Windows), with the same timestamped-backup-before-write policy.
#
# Everything else — flags, defaults, the "never delete local state" uninstall
# policy, the HTTPS-by-default daemon, the four wired clients — is the same
# product as scripts/install-memory-daemon.sh; read that file for the "why".
#
# Usage:
#   pwsh -File scripts/install-memory-daemon.ps1 [flags]
#   pwsh -File scripts/install-memory-daemon.ps1 -Uninstall
#
# Flags:
#   -Embedder <hash|ollama>   Embedding backend (default: prompted, or 'hash' in non-interactive)
#   -Port <port>              HTTP port (default: 18090)
#   -Store <path>             Store directory (default: $env:USERPROFILE\.velesdb-memory)
#   -TlsDir <path>            TLS material (CA + leaf cert) directory (default: $env:USERPROFILE\.velesdb-memory-tls)
#   -OllamaUrl <url>          Ollama endpoint (default: http://localhost:11434)
#   -OllamaModel <model>      Ollama embedding model (default: all-minilm)
#   -Ttl <seconds>            Default TTL for new facts (default: prompted, empty = permanent)
#   -Yes                      Assume yes to interactive prompts (e.g. `ollama pull`)
#   -SkipClient <name>        Skip wiring a client (repeatable): claude-code|claude-desktop|windsurf|devin
#   -SkipCaTrust              Skip trusting the local CA in the CurrentUser\Root store
#   -ForceRestart             Re-register/restart the scheduled task even if already running
#   -FromRelease              Install the prebuilt daemon binary from a GitHub Release archive
#                             instead of building with cargo (see -FromReleaseTag). PowerShell
#                             parameter binding has no "flag with optional inline value" shape
#                             (unlike the shell script's `--from-release[=TAG]`), so the tag is a
#                             separate parameter here.
#   -FromReleaseTag <tag>     Pin the release tag to install from (default: latest
#                             velesdb-memory-v* release). Implies -FromRelease.
#   -SkipChecksum             Install a -FromRelease archive even if its .sha256 can't be
#                             fetched/verified (default: this is a hard error — the checksum
#                             only proves transfer integrity, not authenticity, but skipping it
#                             silently is worse). No effect without -FromRelease.
#   -Uninstall                Remove the scheduled task and all client wiring (store and TLS
#                             material/CA trust are NEVER touched — same "never delete local
#                             state" policy as the store)
#   -Help                     Show this help
# =============================================================================

[CmdletBinding()]
param(
    [ValidateSet('', 'hash', 'ollama')]
    [string]$Embedder = '',

    [int]$Port = 18090,

    [string]$Store = "$env:USERPROFILE\.velesdb-memory",

    # Sibling of the default store, matching velesdb_memory::tls::default_tls_dir —
    # deliberately NOT nested inside Store (independent lifecycles: wiping the store
    # to reset memory shouldn't also invalidate a CA Windows has been told to trust).
    [string]$TlsDir = "$env:USERPROFILE\.velesdb-memory-tls",

    [string]$OllamaUrl = 'http://localhost:11434',

    [string]$OllamaModel = 'all-minilm',

    [string]$Ttl = '',

    [switch]$Yes,

    [string[]]$SkipClient = @(),

    [switch]$SkipCaTrust,

    [switch]$ForceRestart,

    [switch]$FromRelease,

    [string]$FromReleaseTag = '',

    [switch]$SkipChecksum,

    [switch]$Uninstall,

    [switch]$Help
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# Captured HERE, at true script scope, not inside `function Main` further
# down: `$PSBoundParameters` is per-function-scope in PowerShell, so reading
# it from inside a nested function (even one invoked directly from the
# script's own top level, like Main) sees THAT function's own bound
# parameters — Main takes none, so `$PSBoundParameters.ContainsKey('Ttl')`
# there is unconditionally $false, no matter what the caller passed on the
# command line. This bit an earlier version of this script: `-Ttl 3600`
# would still get silently re-prompted-and-overwritten by Resolve-Ttl in an
# interactive session, because the "was -Ttl explicitly passed" check was
# reading the wrong scope's (always-empty) bound-parameters set.
$script:ScriptBoundParameters = $PSBoundParameters

# -FromReleaseTag implies -FromRelease (mirrors the shell script's `--from-release[=TAG]`
# accepting an inline tag without a separate boolean flag).
if ($FromReleaseTag -ne '') { $FromRelease = $true }

# ---- Fixed locations -------------------------------------------------------
$RepoOwner = 'cyberlife-coder'
$RepoName = 'VelesDB'
$RepoSlug = "$RepoOwner/$RepoName"

$TaskFolder = '\VelesDB\'
$TaskName = 'MemoryDaemon'
$ConfigRoot = "$env:LOCALAPPDATA\velesdb-memory"
$LogsDir = "$ConfigRoot\logs"
$WrapperPath = "$ConfigRoot\run-daemon.cmd"
$BinDir = "$env:USERPROFILE\.cargo\bin"
$BinPath = "$BinDir\velesdb-memory.exe"
$DesktopConfig = "$env:APPDATA\Claude\claude_desktop_config.json"
$WindsurfConfig = "$env:USERPROFILE\.codeium\windsurf\mcp_config.json"
# Documented at https://cli.devin.ai/docs/reference/configuration/config-file
# ("%APPDATA%\devin\config.json" on Windows) — unlike most of this ecosystem,
# Devin's own docs give the Windows path explicitly, so this isn't a guess.
$DevinConfig = "$env:APPDATA\devin\config.json"

function Write-Info { param([string]$Message) Write-Host $Message -ForegroundColor Blue }
function Write-Success { param([string]$Message) Write-Host $Message -ForegroundColor Green }
function Write-Warn { param([string]$Message) Write-Host $Message -ForegroundColor Yellow }
function Write-ErrorMsg { param([string]$Message) Write-Host $Message -ForegroundColor Red }

function Show-Usage {
    # Reprint the flag block from this file's own header — keeps `-Help`
    # honest (it can never drift from the comment a reader actually sees).
    $lines = Get-Content -Path $PSCommandPath -TotalCount 70
    ($lines | Select-Object -Skip 1) | ForEach-Object {
        if ($_ -match '^# ?(.*)$') { Write-Host $Matches[1] } else { Write-Host '' }
    }
}

if ($Help) { Show-Usage; exit 0 }

function Test-Interactive {
    # Mirrors the shell script's `[ -t 0 ]`: only prompt when there's a real
    # console attached, not under CI / a redirected pipe.
    -not [System.Console]::IsInputRedirected
}

# Built inline (not via a helper function) deliberately: a function returning
# an empty collection gets unrolled to $null by PowerShell's pipeline output
# semantics (a real footgun here — -SkipClient is usually empty), so this
# constructs the HashSet directly into the script-scope variable instead.
$script:SkipSet = [System.Collections.Generic.HashSet[string]]::new([string[]]$SkipClient)

function Test-ShouldSkip {
    param([string]$Name)
    $script:SkipSet.Contains($Name)
}

# ---- 1. Preflight -----------------------------------------------------------
function Invoke-Preflight {
    if (-not $FromRelease -and -not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Write-ErrorMsg "'cargo' not found — install Rust via https://rustup.rs then relaunch this script (or use -FromRelease to install a prebuilt binary instead)."
        exit 1
    }

    try {
        $script:RepoRoot = (git rev-parse --show-toplevel 2>$null)
        if (-not $script:RepoRoot) { throw 'not a git checkout' }
    } catch {
        Write-ErrorMsg 'Not inside a git checkout of VelesDB — run this script from within the repo.'
        exit 1
    }

    $listener = Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($listener) {
        $proc = Get-Process -Id $listener.OwningProcess -ErrorAction SilentlyContinue
        $procPath = if ($proc) { $proc.Path } else { $null }
        if ($procPath -ne $BinPath) {
            $procName = if ($proc) { $proc.ProcessName } else { 'unknown' }
            Write-ErrorMsg "Port $Port is already in use by another process ($procName, pid $($listener.OwningProcess))."
            Write-ErrorMsg "Re-run with -Port <other-port>, or stop that process first."
            exit 1
        }
    }
}

# ---- 2. Embedder resolution --------------------------------------------------
function Resolve-Embedder {
    if ($script:Embedder -ne '') { return }

    if (Test-Interactive) {
        Write-Host ''
        Write-Info 'Which embedder should velesdb-memory use?'
        Write-Host '  1) hash    (offline, deterministic — default)'
        Write-Host '  2) ollama  (semantic recall — needs a local Ollama)'
        $choice = Read-Host 'Choice [1]'
        switch ($choice) {
            '2' { $script:Embedder = 'ollama' }
            { $_ -in @('', '1') } { $script:Embedder = 'hash' }
            default {
                Write-ErrorMsg "Invalid choice: $choice"
                exit 1
            }
        }
    } else {
        $script:Embedder = 'hash'
        Write-Warn "[velesdb-memory] Using the default 'hash' embedder: deterministic and fully offline, but NOT semantic — recall matches surface form, not meaning. Re-run with -Embedder ollama for real semantic recall."
    }
}

# ---- 2b. TTL resolution -------------------------------------------------------
# $script:TtlExplicitlySet is set once in Main (from $script:ScriptBoundParameters,
# captured at true script scope — see its own comment for why not
# $PSBoundParameters here) and read below.
function Resolve-Ttl {
    if (-not $script:TtlExplicitlySet -and (Test-Interactive)) {
        Write-Host ''
        $script:Ttl = Read-Host 'Default TTL in seconds for new facts (empty = permanent)'
    }

    if ($script:Ttl -ne '' -and $script:Ttl -notmatch '^[0-9]+$') {
        Write-ErrorMsg "-Ttl must be a non-negative integer (seconds), got '$script:Ttl'"
        exit 1
    }
}

# ---- 3. Ollama setup (only when Embedder = ollama) ---------------------------
function Get-NormalizedModelTag {
    param([string]$Model)
    if ($Model -match ':') { $Model } else { "$Model`:latest" }
}

function Initialize-Ollama {
    if ($script:Embedder -ne 'ollama') { return }

    if (-not (Get-Command ollama -ErrorAction SilentlyContinue)) {
        Write-ErrorMsg "'ollama' not found. See https://ollama.com/download for Windows install instructions."
        exit 1
    }

    try {
        $tags = Invoke-RestMethod -Uri "$OllamaUrl/api/tags" -TimeoutSec 2 -ErrorAction Stop
    } catch {
        Write-ErrorMsg "Ollama does not respond on $OllamaUrl — launch the Ollama app or run ``ollama serve``."
        exit 1
    }

    $wanted = Get-NormalizedModelTag -Model $OllamaModel
    $have = @($tags.models | Where-Object { $_.name -eq $wanted }).Count

    if ($have -eq 0) {
        if ($Yes) {
            Write-Warn "Pulling Ollama model '$OllamaModel'..."
            ollama pull $OllamaModel
        } elseif (Test-Interactive) {
            $reply = Read-Host "Model '$OllamaModel' not found locally. Pull it now? [y/N]"
            if ($reply -match '^(y|yes)$') {
                ollama pull $OllamaModel
            } else {
                Write-ErrorMsg "Run this first: ollama pull $OllamaModel"
                exit 1
            }
        } else {
            Write-ErrorMsg "Model '$OllamaModel' not found locally. Run: ollama pull $OllamaModel"
            exit 1
        }
    }
}

# ---- 4. Build (cargo) or install a prebuilt release archive -----------------
function Build-Daemon {
    Write-Warn 'Building velesdb-memory (--features ollama,http)...'
    # Always both features regardless of the runtime embedder choice above:
    # the hash/ollama switch stays a pure VELESDB_MEMORY_EMBEDDER runtime
    # choice, so flipping it later is a restart, never a rebuild.
    cargo install --path "$script:RepoRoot/crates/velesdb-memory" --bin velesdb-memory `
        --features ollama,http --force
    if ($LASTEXITCODE -ne 0) {
        Write-ErrorMsg 'cargo install failed.'
        exit 1
    }
}

function Resolve-LatestReleaseTag {
    # GitHub returns releases newest-first; velesdb-memory ships its own
    # velesdb-memory-vX.Y.Z tags (decoupled from the workspace vX.Y.Z line —
    # see release-memory.yml), created with --latest=false so they never
    # become the repo's overall "Latest release". A plain /releases/latest
    # call would therefore miss them entirely — list and filter instead.
    # HARDENING (same limitation as the shell installer's equivalent lookup):
    # only the first page (100 releases) is scanned; if velesdb-memory ever
    # accumulates more than 100 releases without pruning, pass -FromReleaseTag
    # explicitly instead of relying on this default.
    try {
        $releases = Invoke-RestMethod -Uri "https://api.github.com/repos/$RepoSlug/releases?per_page=100" -TimeoutSec 10
    } catch {
        Write-ErrorMsg "Could not list releases for $RepoSlug`: $($_.Exception.Message)"
        exit 1
    }
    $tag = $releases |
        Where-Object { $_.tag_name -match '^velesdb-memory-v[0-9]+\.[0-9]+\.[0-9]+$' -and -not $_.prerelease } |
        Select-Object -First 1 -ExpandProperty tag_name
    if (-not $tag) {
        Write-ErrorMsg "No published velesdb-memory-vX.Y.Z release found on $RepoSlug — this path needs a release that carries the daemon archive (see the README's -FromRelease note)."
        exit 1
    }
    $tag
}

function Install-FromRelease {
    $tag = if ($FromReleaseTag -ne '') { $FromReleaseTag } else { Resolve-LatestReleaseTag }
    $target = 'x86_64-pc-windows-msvc'
    $asset = "velesdb-memory-daemon-$target.zip"
    $baseUrl = "https://github.com/$RepoSlug/releases/download/$tag"

    Write-Warn "Installing velesdb-memory from release $tag ($asset)..."

    $tempDir = Join-Path $env:TEMP "velesdb-memory-daemon-$tag"
    New-Item -ItemType Directory -Force -Path $tempDir | Out-Null
    $archivePath = Join-Path $tempDir $asset
    $checksumPath = "$archivePath.sha256"

    try {
        Invoke-WebRequest -Uri "$baseUrl/$asset" -OutFile $archivePath -ErrorAction Stop
    } catch {
        Write-ErrorMsg "Failed to download $baseUrl/$asset — this tag may predate the daemon archive (added in the release-memory.yml workflow after 0.11.0). $($_.Exception.Message)"
        Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
        exit 1
    }

    # Blocking by default: a checksum that can't be fetched/verified is
    # treated the same as a mismatch (installing an unverified binary
    # silently is worse than a loud failure). -SkipChecksum is the explicit
    # opt-out. Note this only proves TRANSFER integrity (the bytes weren't
    # corrupted/truncated in flight) — it is not a cryptographic signature,
    # so it does not by itself prove the archive is authentic; the README's
    # "Installing the daemon without a Rust toolchain" section says so too.
    if ($SkipChecksum) {
        Write-Warn "Skipping checksum verification (-SkipChecksum) — the downloaded archive's integrity will not be checked."
    } else {
        try {
            Invoke-WebRequest -Uri "$baseUrl/$asset.sha256" -OutFile $checksumPath -ErrorAction Stop
        } catch {
            Write-ErrorMsg "Could not fetch the checksum for $asset ($baseUrl/$asset.sha256) — aborting rather than installing an unverified binary. Pass -SkipChecksum to install anyway (not recommended)."
            Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
            exit 1
        }
        $expected = (Get-Content $checksumPath -Raw).Split(' ')[0].Trim().ToLowerInvariant()
        $actual = (Get-FileHash -Path $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($expected -ne $actual) {
            Write-ErrorMsg "Checksum mismatch for $asset — expected $expected, got $actual. Aborting (the archive may be corrupt or tampered)."
            Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
            exit 1
        }
        Write-Success 'Checksum verified (transfer integrity — not a signature of authenticity).'
    }

    $extractDir = Join-Path $tempDir 'extracted'
    Expand-Archive -Path $archivePath -DestinationPath $extractDir -Force

    $exe = Get-ChildItem -Path $extractDir -Filter 'velesdb-memory.exe' -Recurse | Select-Object -First 1
    if (-not $exe) {
        Write-ErrorMsg "velesdb-memory.exe not found inside $asset — unexpected archive layout."
        Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
        exit 1
    }

    New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
    Copy-Item -Path $exe.FullName -Destination $BinPath -Force
    Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue

    Write-Success "Installed $BinPath from $tag$(if ($SkipChecksum) { ' (unverified — -SkipChecksum)' })."
}

# ---- 5. Scheduled Task daemon -------------------------------------------------
function New-DaemonWrapper {
    New-Item -ItemType Directory -Force -Path $ConfigRoot | Out-Null
    New-Item -ItemType Directory -Force -Path $LogsDir | Out-Null

    # Empty TTL means "permanent" (VELESDB_MEMORY_DEFAULT_TTL unset) — matches
    # the server's own default, so the line is omitted entirely rather than
    # set to an empty value. Quoted (`set "VAR=value"`, not bare `set VAR=value`)
    # so a value containing `&`, `|`, `<`, `>`, `^`, or trailing spaces — a
    # store/TLS path or Ollama URL isn't guaranteed to avoid all of those —
    # doesn't get parsed as a batch operator instead of literal text.
    $ttlLine = if ($script:Ttl -ne '') { "set `"VELESDB_MEMORY_DEFAULT_TTL=$script:Ttl`"" } else { '' }

    # A Scheduled Task action carries no environment block of its own, so this
    # wrapper is how the daemon's env vars actually reach the process — the
    # task just launches this .cmd. Output is redirected here too, since
    # Register-ScheduledTask has no stdout/stderr-path equivalent to launchd's
    # StandardOutPath/StandardErrorPath.
    $content = @"
@echo off
chcp 65001 >nul
set "VELESDB_MEMORY_PATH=$script:Store"
set "VELESDB_MEMORY_TLS_DIR=$script:TlsDir"
set "VELESDB_MEMORY_EMBEDDER=$script:Embedder"
set "VELESDB_MEMORY_OLLAMA_URL=$OllamaUrl"
set "VELESDB_MEMORY_OLLAMA_MODEL=$OllamaModel"
$ttlLine
"$BinPath" --http --http-port $Port >> "$LogsDir\daemon.out.log" 2>> "$LogsDir\daemon.err.log"
"@

    # cmd.exe decodes a batch file using its active code page, which by
    # default is NOT UTF-8 — a non-ASCII $env:USERPROFILE (an accented
    # Windows username, e.g. a French "Céline") would otherwise get
    # corrupted when this file's UTF-8 bytes are misread, breaking every
    # `set` line that embeds $Store/$TlsDir/$BinPath. `chcp 65001` (above,
    # first line after @echo off — that ordering matters) switches cmd.exe's
    # active code page to UTF-8 for the rest of THIS script's execution
    # before any non-ASCII line is parsed. That only works paired with an
    # actual UTF-8-encoded file — hence -Encoding utf8NoBOM below (no BOM:
    # a BOM before `@echo off` is a known way to corrupt a batch file's
    # first line on some cmd.exe versions). cmd.exe also expects CRLF line
    # endings, so normalize explicitly rather than relying on how this
    # script's own here-string literal happens to be checked out.
    $content = ($content -replace "`r`n", "`n") -replace "`n", "`r`n"
    Set-Content -Path $WrapperPath -Value $content -Encoding utf8NoBOM -NoNewline
}

function Wait-DaemonHealth {
    param([string]$CaCertPath)
    Write-Warn 'Waiting for the daemon to answer /health...'
    $curl = Get-Command curl.exe -ErrorAction SilentlyContinue
    if (-not $curl) {
        Write-Warn 'curl.exe not found (expected on Windows 10 1803+/11) — skipping the active health check. Verify manually once the daemon starts.'
        return
    }
    for ($waited = 0; $waited -lt 5; $waited++) {
        & $curl.Source -fsS --max-time 1 --cacert $CaCertPath "https://127.0.0.1:$Port/health" *> $null
        if ($LASTEXITCODE -eq 0) { return }
        Start-Sleep -Seconds 1
    }
    Write-Warn "No response from /health within 5s — check $LogsDir\daemon.err.log"
}

function Set-Daemon {
    $script:DaemonAlreadyRunning = $false
    $caCert = "$script:TlsDir\ca-cert.pem"

    $existingTask = Get-ScheduledTask -TaskName $TaskName -TaskPath $TaskFolder -ErrorAction SilentlyContinue

    if ($existingTask -and -not $ForceRestart) {
        Write-Success "$TaskFolder$TaskName is already registered — skipping (pass -ForceRestart to reload)."
        $script:DaemonAlreadyRunning = $true
        if ($existingTask.State -ne 'Running') {
            Start-ScheduledTask -TaskName $TaskName -TaskPath $TaskFolder
        }
        # Still (re-)attempt CA trust even when the task itself isn't
        # restarted — a task can be "already registered" from a run that
        # predates the CA existing yet, which would otherwise leave the
        # local CA permanently untrusted.
        Wait-DaemonHealth -CaCertPath $caCert
        Enable-LocalCaTrust -CaCertPath $caCert
        return
    }

    if ($existingTask) {
        Write-Warn "-ForceRestart: unregistering the existing $TaskFolder$TaskName..."
        Unregister-ScheduledTask -TaskName $TaskName -TaskPath $TaskFolder -Confirm:$false -ErrorAction SilentlyContinue
    }

    New-DaemonWrapper

    $action = New-ScheduledTaskAction -Execute $WrapperPath
    $trigger = New-ScheduledTaskTrigger -AtLogOn
    $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
        -StartWhenAvailable -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) `
        -ExecutionTimeLimit ([TimeSpan]::Zero)
    # RunLevel Limited: a *user*-level task, deliberately not "Highest" — this
    # installer never needs, and never asks for, admin rights.
    $principal = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" -LogonType Interactive -RunLevel Limited

    Register-ScheduledTask -TaskName $TaskName -TaskPath $TaskFolder -Action $action `
        -Trigger $trigger -Settings $settings -Principal $principal -Force | Out-Null

    Start-ScheduledTask -TaskName $TaskName -TaskPath $TaskFolder

    Wait-DaemonHealth -CaCertPath $caCert
    Enable-LocalCaTrust -CaCertPath $caCert
}

# Run a script block with a hard wall-clock timeout, mirroring the shell
# script's `run_with_timeout` — used below because `certutil -addstore` can
# in principle block on a confirmation dialog, and this script must never
# hang forever waiting on it.
function Invoke-WithTimeout {
    param(
        [int]$Seconds,
        [string]$FilePath,
        [string[]]$ArgumentList
    )
    $proc = Start-Process -FilePath $FilePath -ArgumentList $ArgumentList -NoNewWindow -PassThru
    if (-not $proc.WaitForExit($Seconds * 1000)) {
        try { $proc.Kill() } catch {}
        return $false
    }
    return $proc.ExitCode -eq 0
}

# ---- 5b. Trust the local CA in the CurrentUser\Root store --------------------
function Enable-LocalCaTrust {
    param([string]$CaCertPath)

    if ($SkipCaTrust) {
        Write-Warn 'Skipping CA trust (-SkipCaTrust).'
        return
    }
    if (-not (Test-Path $CaCertPath)) {
        Write-Warn "No CA certificate at $CaCertPath (daemon may not have started — see the /health warning above) — skipping CA trust."
        return
    }

    # Ground-truth idempotency check first, same as the macOS script: if a
    # strict HTTPS request already succeeds against the SYSTEM/user trust
    # store, the CA is already trusted — skip re-running certutil.
    $curl = Get-Command curl.exe -ErrorAction SilentlyContinue
    if ($curl) {
        & $curl.Source -fsS --max-time 2 "https://127.0.0.1:$Port/health" *> $null
        if ($LASTEXITCODE -eq 0) {
            Write-Success 'Local CA already trusted (strict HTTPS request to the daemon succeeded).'
            return
        }
    }

    # Belt-and-braces: also check by thumbprint before importing, so a
    # `certutil` failure that still left the cert present doesn't loop.
    try {
        $cert = [System.Security.Cryptography.X509Certificates.X509Certificate2]::new($CaCertPath)
        $existing = Get-ChildItem Cert:\CurrentUser\Root | Where-Object { $_.Thumbprint -eq $cert.Thumbprint }
        if ($existing) {
            Write-Success 'Local CA already present in Cert:\CurrentUser\Root (by thumbprint) — skipping import.'
            return
        }
    } catch {
        Write-Warn "Could not read $CaCertPath as a certificate to check its thumbprint — attempting import anyway."
    }

    Write-Host ''
    Write-Info "Trusting the local CA in your user certificate store ($CaCertPath)..."
    Write-Warn '   Windows may show a security warning dialog — approve it within 60s. Without this,'
    Write-Warn '   HTTPS clients that verify certificates strictly (browsers, plain curl) will reject'
    Write-Warn '   this daemon''s certificate until you trust it, here or by hand later.'

    # certutil (not Import-Certificate) — accepts the daemon's PEM output
    # directly and natively targets the per-user store with `-user`, no admin
    # rights needed, and behaves consistently across PowerShell versions.
    $ok = Invoke-WithTimeout -Seconds 60 -FilePath 'certutil.exe' -ArgumentList @('-addstore', '-user', 'Root', "`"$CaCertPath`"")
    if ($ok) {
        Write-Success 'Local CA trusted in Cert:\CurrentUser\Root.'
    } else {
        Write-Warn '   Could not confirm the CA trust automatically (no response within 60s, or the'
        Write-Warn '   command failed). The daemon is still up and serving HTTPS — this only affects'
        Write-Warn '   clients that verify certificates strictly. Run this yourself to finish:'
        Write-Host "     certutil -addstore -user Root `"$CaCertPath`""
    }
}

# ---- 6. Client wiring ---------------------------------------------------------
function Set-ClaudeCodeClient {
    if (Test-ShouldSkip 'claude-code') {
        Write-Warn 'Skipping Claude Code (-SkipClient).'
        return
    }
    if (-not (Get-Command claude -ErrorAction SilentlyContinue)) {
        Write-Warn "'claude' CLI not found — skipping Claude Code wiring."
        return
    }

    claude mcp remove velesdb-memory -s user *> $null
    claude mcp add --transport http --scope user velesdb-memory "https://127.0.0.1:$Port/mcp" *> $null
    if ($LASTEXITCODE -eq 0) {
        Write-Success "Claude Code wired (user scope) -> https://127.0.0.1:$Port/mcp"
        Write-Warn '   Note: project/local-scope entries (if any) are not touched — check with `claude mcp list`.'
        Write-Warn '   Note: Node-based tools don''t always consult the Windows certificate store for TLS'
        Write-Warn '   trust. If Claude Code reports a certificate error despite the CA trust step above, set:'
        Write-Warn "     `$env:NODE_EXTRA_CA_CERTS = `"$script:TlsDir\ca-cert.pem`""
    } else {
        Write-ErrorMsg 'Failed to wire Claude Code.'
    }
}

# Claude Desktop is a DIFFERENT mechanism than every other client here:
# claude_desktop_config.json never reads a url/type:"http" entry (confirmed on
# macOS, same binary/config format on Windows) — the only way to wire Desktop
# to the daemon is its own UI. This prints that instruction instead of
# touching the config file, same as the shell script.
function Set-ClaudeDesktopClient {
    if (Test-ShouldSkip 'claude-desktop') {
        Write-Warn 'Skipping Claude Desktop (-SkipClient).'
        return
    }
    Write-Host ''
    Write-Info 'Claude Desktop — different mechanism than every other client here:'
    Write-Warn "   its config file ($DesktopConfig) does not support HTTP (a url/type:`"http`" entry"
    Write-Warn '   there is silently ignored). Add it yourself, once, via the UI instead:'
    Write-Warn '   Settings -> Connectors -> Add custom connector, then paste:'
    Write-Host "     https://127.0.0.1:$Port/mcp"
    Write-Warn '   No API key needed (loopback only) — requires the CA-trust step above to have succeeded.'
    Write-Warn '   Prefer not to use the Connectors UI? A stdio fallback still works — see the README''s'
    Write-Warn "   `"Configure your client`" section (use a DIFFERENT VELESDB_MEMORY_PATH than $script:Store,"
    Write-Warn '   or the fallback process and the daemon will fight over the same lock).'
}

# Set-JsonClient NAME CONFIG_PATH MUTATOR REQUIRE_EXISTING_DIR
# REQUIRE_EXISTING_DIR skips (rather than creates) the client's config
# directory when absent — used for Devin, whose directory only exists if the
# CLI itself is installed; Windsurf's is created if missing (same split as
# the shell script's wire_json_client callers).
function Set-JsonClient {
    param(
        [string]$Name,
        [string]$ConfigPath,
        [scriptblock]$Mutator,
        [bool]$RequireExistingDir
    )
    if (Test-ShouldSkip $Name) {
        Write-Warn "Skipping $Name (-SkipClient)."
        return
    }

    $configDir = Split-Path -Path $ConfigPath -Parent
    if ($RequireExistingDir -and -not (Test-Path $configDir)) {
        Write-Warn "$Name not detected (no $configDir) — skipping."
        return
    }
    New-Item -ItemType Directory -Force -Path $configDir | Out-Null

    $existed = Test-Path $ConfigPath
    if (-not $existed) {
        Set-Content -Path $ConfigPath -Value '{}'
    }

    try {
        $raw = Get-Content -Path $ConfigPath -Raw
        $json = $raw | ConvertFrom-Json -ErrorAction Stop
    } catch {
        Write-ErrorMsg "$ConfigPath is not valid JSON — fix or remove it manually, then re-run."
        return
    }

    if ($existed) {
        $backup = "$ConfigPath.bak.$([DateTimeOffset]::UtcNow.ToUnixTimeSeconds())"
        Copy-Item -Path $ConfigPath -Destination $backup
    }

    try {
        $json = & $Mutator $json
        ($json | ConvertTo-Json -Depth 10) | Set-Content -Path $ConfigPath
        Write-Success "$Name wired -> $ConfigPath"
    } catch {
        Write-ErrorMsg "Failed to update $ConfigPath`: $($_.Exception.Message)"
    }
}

function Set-WindsurfClient {
    Set-JsonClient -Name 'windsurf' -ConfigPath $WindsurfConfig -RequireExistingDir $false -Mutator {
        param($json)
        if (-not $json.PSObject.Properties['mcpServers']) {
            $json | Add-Member -NotePropertyName 'mcpServers' -NotePropertyValue ([PSCustomObject]@{})
        }
        $entry = [PSCustomObject]@{ serverUrl = "https://127.0.0.1:$Port/mcp" }
        if ($json.mcpServers.PSObject.Properties['velesdb-memory']) {
            $json.mcpServers.'velesdb-memory' = $entry
        } else {
            $json.mcpServers | Add-Member -NotePropertyName 'velesdb-memory' -NotePropertyValue $entry
        }
        $json
    }
}

# Devin CLI's config wraps mcpServers in a top-level {"version": 1, ...}
# envelope (unlike every other client here) — version is set only if absent,
# so a re-run never clobbers a newer schema version Devin itself might have
# written.
function Set-DevinClient {
    Set-JsonClient -Name 'devin' -ConfigPath $DevinConfig -RequireExistingDir $true -Mutator {
        param($json)
        if (-not $json.PSObject.Properties['version']) {
            $json | Add-Member -NotePropertyName 'version' -NotePropertyValue 1
        }
        if (-not $json.PSObject.Properties['mcpServers']) {
            $json | Add-Member -NotePropertyName 'mcpServers' -NotePropertyValue ([PSCustomObject]@{})
        }
        $entry = [PSCustomObject]@{ url = "https://127.0.0.1:$Port/mcp"; transport = 'http' }
        if ($json.mcpServers.PSObject.Properties['velesdb-memory']) {
            $json.mcpServers.'velesdb-memory' = $entry
        } else {
            $json.mcpServers | Add-Member -NotePropertyName 'velesdb-memory' -NotePropertyValue $entry
        }
        $json
    }
}

# ---- 7. Uninstall --------------------------------------------------------------
function Invoke-Uninstall {
    Write-Warn 'Uninstalling the velesdb-memory daemon and client wiring...'

    Unregister-ScheduledTask -TaskName $TaskName -TaskPath $TaskFolder -Confirm:$false -ErrorAction SilentlyContinue
    Remove-Item -Path $WrapperPath -Force -ErrorAction SilentlyContinue

    if (Get-Command claude -ErrorAction SilentlyContinue) {
        claude mcp remove velesdb-memory -s user *> $null
    }

    foreach ($cfg in @($DesktopConfig, $WindsurfConfig, $DevinConfig)) {
        if (Test-Path $cfg) {
            try {
                $json = Get-Content -Path $cfg -Raw | ConvertFrom-Json -ErrorAction Stop
                if ($json.PSObject.Properties['mcpServers'] -and $json.mcpServers.PSObject.Properties['velesdb-memory']) {
                    $json.mcpServers.PSObject.Properties.Remove('velesdb-memory')
                    ($json | ConvertTo-Json -Depth 10) | Set-Content -Path $cfg
                }
            } catch {
                Write-Warn "Skipped $cfg (not valid JSON)."
            }
        }
    }

    Write-Success "Uninstalled. The store ($Store by default) and the TLS material/CA ($TlsDir by default)"
    Write-Success '   were both left untouched — same policy as the store: nothing local is ever deleted by'
    Write-Success '   an uninstall. This also means the certificate trust you approved earlier stays valid, so a'
    Write-Success '   future reinstall needs no new trust prompt. Remove either by hand if you want them gone.'
}

# ---- 8. Summary -----------------------------------------------------------------
function Show-Summary {
    Write-Host ''
    Write-Info '==============================================='
    Write-Info '  velesdb-memory daemon — summary'
    Write-Info '==============================================='
    Write-Host "  Embedder:  $script:Embedder"
    Write-Host "  Port:      $Port"
    Write-Host "  Store:     $script:Store"
    Write-Host "  TLS CA:    $script:TlsDir\ca-cert.pem"
    Write-Host "  TTL:       $(if ($script:Ttl -ne '') { $script:Ttl } else { 'permanent (no expiry)' })"

    $curl = Get-Command curl.exe -ErrorAction SilentlyContinue
    $up = $false
    if ($curl) {
        & $curl.Source -fsS --max-time 1 --cacert "$script:TlsDir\ca-cert.pem" "https://127.0.0.1:$Port/health" *> $null
        $up = ($LASTEXITCODE -eq 0)
    }
    if ($up) {
        $suffix = if ($script:DaemonAlreadyRunning) { ' (already registered, not restarted)' } else { '' }
        Write-Host "  Daemon:    " -NoNewline
        Write-Success "running -> https://127.0.0.1:$Port/mcp$suffix"
    } else {
        Write-Host "  Daemon:    " -NoNewline
        Write-Warn "not confirmed up — check $LogsDir\daemon.err.log"
    }

    foreach ($client in @('claude-code', 'claude-desktop', 'windsurf', 'devin')) {
        if (Test-ShouldSkip $client) {
            Write-Host "  $client`: skipped (-SkipClient)"
        } else {
            Write-Host "  $client`: wired (see log above for details/warnings)"
        }
    }
    Write-Host ''
}

# ---- Main -----------------------------------------------------------------------
function Main {
    if ($Uninstall) {
        Invoke-Uninstall
        exit 0
    }

    # Read from $script:ScriptBoundParameters (captured at true script scope,
    # above the param() block) — NOT $PSBoundParameters here, which inside
    # this function would be Main's own (always-empty) bound parameters. See
    # the comment where $script:ScriptBoundParameters is assigned.
    $script:TtlExplicitlySet = $script:ScriptBoundParameters.ContainsKey('Ttl')

    Invoke-Preflight
    Resolve-Embedder
    Resolve-Ttl
    Initialize-Ollama
    if ($FromRelease) { Install-FromRelease } else { Build-Daemon }
    Set-Daemon
    Set-ClaudeCodeClient
    Set-ClaudeDesktopClient
    Set-WindsurfClient
    Set-DevinClient
    Show-Summary
}

Main
