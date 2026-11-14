# =============================================================================
# VelesDB install wizard — one-line installer for Windows
# =============================================================================
#   irm https://raw.githubusercontent.com/cyberlife-coder/VelesDB/main/scripts/install-wizard.ps1 | iex
#
# Wizard flags need the script-block form:
#   & ([scriptblock]::Create((irm https://raw.githubusercontent.com/cyberlife-coder/VelesDB/main/scripts/install-wizard.ps1))) --dry-run
#
# Finds the newest velesdb-memory-vX.Y.Z release that carries
# velesdb-install-wizard-x86_64-pc-windows-msvc.exe (built by
# release-memory.yml), verifies it against its .sha256 sidecar and runs it.
# Windows on ARM runs the same x64 build under the OS's x64 emulation.
# The checksum proves transfer integrity, not authenticity: the sidecar comes
# from the same release. The exe also carries a publisher signature, which
# scripts/validate_package.py reads.
#
# Dot-sourcing this file defines its functions without running anything.
# =============================================================================

$WizardRepo = 'cyberlife-coder/VelesDB'
$WizardApi = "https://api.github.com/repos/$WizardRepo/releases?per_page=100"
$WizardDownload = "https://github.com/$WizardRepo/releases/download"
# How many velesdb-memory releases to look back through for a wizard.
$WizardMaxTags = 10

function Get-WizardTarget([string]$Architecture = $env:PROCESSOR_ARCHITECTURE) {
    # x64 and ARM64 Windows both run the x64 build.
    switch ($Architecture) {
        'AMD64' { 'x86_64-pc-windows-msvc' }
        'ARM64' { 'x86_64-pc-windows-msvc' }
        default { throw "no install wizard build for Windows/$Architecture" }
    }
}

function Get-MemoryTags {
    # velesdb-memory-vX.Y.Z, newest first; drafts, prereleases and suffixed tags skipped.
    $releases = Invoke-RestMethod -Uri $WizardApi
    @($releases |
        Where-Object { -not $_.draft -and -not $_.prerelease -and $_.tag_name -match '^velesdb-memory-v\d+\.\d+\.\d+$' } |
        Select-Object -First $WizardMaxTags |
        ForEach-Object { $_.tag_name })
}

function Get-VerifiedWizard([string]$Directory) {
    # Download the newest release's wizard exe and sidecar into $Directory and
    # return the exe path; throw on a checksum mismatch or when none exists.
    $asset = "velesdb-install-wizard-$(Get-WizardTarget).exe"
    foreach ($tag in Get-MemoryTags) {
        $exe = Join-Path $Directory $asset
        try {
            Invoke-WebRequest -Uri "$WizardDownload/$tag/$asset" -OutFile $exe -UseBasicParsing
        } catch {
            continue
        }
        $sidecar = "$exe.sha256"
        try {
            Invoke-WebRequest -Uri "$WizardDownload/$tag/$asset.sha256" -OutFile $sidecar -UseBasicParsing
        } catch {
            throw "$asset in $tag has no .sha256 sidecar - refusing to run an unverified wizard"
        }
        $expected = ((Get-Content -Raw $sidecar).Trim() -split '\s+')[0].ToLowerInvariant()
        $actual = (Get-FileHash -Algorithm SHA256 -Path $exe).Hash.ToLowerInvariant()
        if ($expected -ne $actual) {
            throw "sha256 mismatch for $asset in $tag (expected $expected, got $actual) - the download may be corrupt or tampered"
        }
        Write-Host "velesdb: running the install wizard from $tag (sha256 OK)"
        return $exe
    }
    throw "no release of $WizardRepo carries $asset yet. From a checkout: python scripts\velesdb_install_wizard.py"
}

function Install-VelesDBWizard {
    $ErrorActionPreference = 'Stop'
    # Windows PowerShell 5.1, the default host of `irm | iex`, may not offer
    # TLS 1.2, which GitHub requires; its progress bar slows downloads tenfold.
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    $ProgressPreference = 'SilentlyContinue'
    $work = Join-Path ([System.IO.Path]::GetTempPath()) ("velesdb-wizard-" + [guid]::NewGuid())
    New-Item -ItemType Directory -Path $work | Out-Null
    try {
        $exe = Get-VerifiedWizard -Directory $work
        & $exe @args
        $code = $LASTEXITCODE
    } finally {
        Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
    }
    if ($code -ne 0) { throw "the install wizard exited $code" }
}

if ($MyInvocation.InvocationName -ne '.') {
    Install-VelesDBWizard @args
}
