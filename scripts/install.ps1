# =============================================================================
# VelesDB Installer Script for Windows
# =============================================================================
# One-liner installation:
#   irm https://raw.githubusercontent.com/cyberlife-coder/VelesDB/main/scripts/install.ps1 | iex
# =============================================================================

$ErrorActionPreference = "Stop"

# Configuration
$Repo = "cyberlife-coder/VelesDB"
$InstallDir = if ($env:VELESDB_INSTALL_DIR) { $env:VELESDB_INSTALL_DIR } else { "$env:LOCALAPPDATA\VelesDB" }

Write-Host ""
Write-Host "╔═══════════════════════════════════════════════════════════════╗" -ForegroundColor Cyan
Write-Host "║                                                               ║" -ForegroundColor Cyan
Write-Host "║   🐺 VelesDB Installer for Windows                            ║" -ForegroundColor Cyan
Write-Host "║   High-Performance Vector Database for AI Applications        ║" -ForegroundColor Cyan
Write-Host "║                                                               ║" -ForegroundColor Cyan
Write-Host "╚═══════════════════════════════════════════════════════════════╝" -ForegroundColor Cyan
Write-Host ""

# Get latest release version
function Get-LatestVersion {
    $releases = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest"
    return $releases.tag_name -replace '^v', ''
}

# Download and install
function Install-VelesDB {
    Write-Host "🔍 Fetching latest version..." -ForegroundColor Yellow
    $version = Get-LatestVersion
    
    if (-not $version) {
        Write-Host "❌ Failed to get latest version" -ForegroundColor Red
        exit 1
    }
    
    Write-Host "📦 Latest version: v$version" -ForegroundColor Green
    
    $archiveName = "velesdb-windows-x86_64.zip"
    $downloadUrl = "https://github.com/$Repo/releases/download/v$version/$archiveName"
    
    # Create install directory
    Write-Host "📁 Creating directory: $InstallDir" -ForegroundColor Yellow
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    
    # Download
    Write-Host "⬇️  Downloading VelesDB v$version..." -ForegroundColor Yellow
    $tempFile = Join-Path $env:TEMP $archiveName
    Invoke-WebRequest -Uri $downloadUrl -OutFile $tempFile
    
    # Extract
    Write-Host "📦 Extracting..." -ForegroundColor Yellow
    Expand-Archive -Path $tempFile -DestinationPath $InstallDir -Force
    
    # Cleanup
    Remove-Item $tempFile -Force
    
    # Verify installation
    $exePath = Join-Path $InstallDir "velesdb.exe"
    if (Test-Path $exePath) {
        Write-Host "✅ VelesDB installed successfully!" -ForegroundColor Green
    } else {
        Write-Host "❌ Installation failed" -ForegroundColor Red
        exit 1
    }
}

# Add to PATH
function Add-ToPath {
    $currentPath = [Environment]::GetEnvironmentVariable("Path", "User")
    
    if ($currentPath -notlike "*$InstallDir*") {
        Write-Host ""
        Write-Host "📝 Adding to PATH..." -ForegroundColor Yellow
        
        $newPath = "$currentPath;$InstallDir"
        [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
        $env:Path = "$env:Path;$InstallDir"
        
        Write-Host "✅ Added to PATH. Restart your terminal for changes to take effect." -ForegroundColor Green
    }
}

# Print usage
function Show-Usage {
    Write-Host ""
    Write-Host "🚀 Quick Start:" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "   # Start the server"
    Write-Host "   velesdb-server"
    Write-Host ""
    Write-Host "   # Open interactive CLI"
    Write-Host "   velesdb"
    Write-Host ""
    Write-Host "   # Create a collection"
    Write-Host '   curl -X POST http://localhost:8080/collections `'
    Write-Host '     -d ''{"name": "docs", "dimension": 768, "metric": "cosine"}'''
    Write-Host ""
    Write-Host "📚 Documentation: https://github.com/$Repo#readme" -ForegroundColor Cyan
    Write-Host ""
}

# Main
Install-VelesDB
Add-ToPath
Show-Usage
