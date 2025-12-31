# VelesDB Migration - Test with Real Data
# Usage: .\scripts\test-with-real-data.ps1
#
# Environment variables required:
#   $env:SUPABASE_URL = "https://YOUR_PROJECT.supabase.co"
#   $env:SUPABASE_SERVICE_KEY = "your-service-key"
#   $env:SUPABASE_TABLE = "your_table_name"

param(
    [switch]$IntegrationTests,
    [switch]$Benchmarks,
    [switch]$FullMigration,
    [switch]$All
)

$ErrorActionPreference = "Stop"

Write-Host "╔══════════════════════════════════════════════════════════════╗" -ForegroundColor Cyan
Write-Host "║         VelesDB Migration - Real Data Testing                 ║" -ForegroundColor Cyan
Write-Host "╚══════════════════════════════════════════════════════════════╝" -ForegroundColor Cyan
Write-Host ""

# Check environment variables
if (-not $env:SUPABASE_URL) {
    Write-Host "❌ SUPABASE_URL not set" -ForegroundColor Red
    Write-Host "   Set it with: `$env:SUPABASE_URL = 'https://YOUR_PROJECT.supabase.co'" -ForegroundColor Yellow
    exit 1
}

if (-not $env:SUPABASE_SERVICE_KEY) {
    Write-Host "❌ SUPABASE_SERVICE_KEY not set" -ForegroundColor Red
    Write-Host "   Set it with: `$env:SUPABASE_SERVICE_KEY = 'your-service-key'" -ForegroundColor Yellow
    exit 1
}

if (-not $env:SUPABASE_TABLE) {
    Write-Host "❌ SUPABASE_TABLE not set" -ForegroundColor Red
    Write-Host "   Set it with: `$env:SUPABASE_TABLE = 'your_table_name'" -ForegroundColor Yellow
    exit 1
}
$table = $env:SUPABASE_TABLE
Write-Host "✅ Environment configured:" -ForegroundColor Green
Write-Host "   URL: $($env:SUPABASE_URL)" -ForegroundColor Gray
Write-Host "   Table: $table" -ForegroundColor Gray
Write-Host ""

# Navigate to project root
$scriptPath = Split-Path -Parent $MyInvocation.MyCommand.Path
$projectRoot = Split-Path -Parent (Split-Path -Parent $scriptPath)
Set-Location $projectRoot

if ($All) {
    $IntegrationTests = $true
    $Benchmarks = $true
    $FullMigration = $true
}

# 1. Run Integration Tests
if ($IntegrationTests) {
    Write-Host "═══════════════════════════════════════════════════════════════" -ForegroundColor Blue
    Write-Host "🧪 Running Integration Tests..." -ForegroundColor Blue
    Write-Host "═══════════════════════════════════════════════════════════════" -ForegroundColor Blue
    Write-Host ""
    
    cargo test -p velesdb-migrate --test integration_test -- --ignored --nocapture
    
    if ($LASTEXITCODE -eq 0) {
        Write-Host ""
        Write-Host "✅ Integration tests passed!" -ForegroundColor Green
    } else {
        Write-Host ""
        Write-Host "❌ Integration tests failed!" -ForegroundColor Red
        exit 1
    }
    Write-Host ""
}

# 2. Run Benchmarks
if ($Benchmarks) {
    Write-Host "═══════════════════════════════════════════════════════════════" -ForegroundColor Magenta
    Write-Host "📊 Running Benchmarks..." -ForegroundColor Magenta
    Write-Host "═══════════════════════════════════════════════════════════════" -ForegroundColor Magenta
    Write-Host ""
    
    cargo bench -p velesdb-migrate
    
    Write-Host ""
    Write-Host "✅ Benchmarks completed! Results in target/criterion/" -ForegroundColor Green
    Write-Host ""
}

# 3. Full Migration Test
if ($FullMigration) {
    Write-Host "═══════════════════════════════════════════════════════════════" -ForegroundColor Yellow
    Write-Host "🚀 Running Full Migration Test..." -ForegroundColor Yellow
    Write-Host "═══════════════════════════════════════════════════════════════" -ForegroundColor Yellow
    Write-Host ""
    
    # Create temp directory for test
    $testDir = Join-Path $env:TEMP "velesdb_migration_test_$(Get-Date -Format 'yyyyMMdd_HHmmss')"
    New-Item -ItemType Directory -Path $testDir -Force | Out-Null
    
    Write-Host "📁 Test directory: $testDir" -ForegroundColor Gray
    Write-Host ""
    
    # Step 1: Detect schema
    Write-Host "1️⃣ Detecting schema..." -ForegroundColor Cyan
    $configFile = Join-Path $testDir "migration.yaml"
    
    & .\target\release\velesdb-migrate.exe detect `
        --source supabase `
        --url $env:SUPABASE_URL `
        --collection $table `
        --api-key $env:SUPABASE_SERVICE_KEY `
        --output $configFile `
        --dest-path (Join-Path $testDir "velesdb_data")
    
    if ($LASTEXITCODE -ne 0) {
        Write-Host "❌ Schema detection failed!" -ForegroundColor Red
        exit 1
    }
    
    Write-Host ""
    Write-Host "📝 Generated config:" -ForegroundColor Gray
    Get-Content $configFile | Write-Host -ForegroundColor DarkGray
    Write-Host ""
    
    # Step 2: Validate config
    Write-Host "2️⃣ Validating configuration..." -ForegroundColor Cyan
    & .\target\release\velesdb-migrate.exe validate --config $configFile
    
    if ($LASTEXITCODE -ne 0) {
        Write-Host "❌ Validation failed!" -ForegroundColor Red
        exit 1
    }
    Write-Host "✅ Configuration valid!" -ForegroundColor Green
    Write-Host ""
    
    # Step 3: Show schema
    Write-Host "3️⃣ Fetching source schema..." -ForegroundColor Cyan
    & .\target\release\velesdb-migrate.exe schema --config $configFile
    Write-Host ""
    
    # Step 4: Dry run
    Write-Host "4️⃣ Dry run (no data written)..." -ForegroundColor Cyan
    & .\target\release\velesdb-migrate.exe run --config $configFile --dry-run
    
    if ($LASTEXITCODE -eq 0) {
        Write-Host "✅ Dry run successful!" -ForegroundColor Green
    } else {
        Write-Host "⚠️ Dry run had issues" -ForegroundColor Yellow
    }
    Write-Host ""
    
    # Ask before actual migration
    Write-Host "═══════════════════════════════════════════════════════════════" -ForegroundColor Yellow
    $confirm = Read-Host "Run actual migration? This will import data to $testDir/velesdb_data (y/N)"
    
    if ($confirm -eq "y" -or $confirm -eq "Y") {
        Write-Host ""
        Write-Host "5️⃣ Running migration..." -ForegroundColor Cyan
        
        $startTime = Get-Date
        & .\target\release\velesdb-migrate.exe run --config $configFile
        $endTime = Get-Date
        $duration = $endTime - $startTime
        
        if ($LASTEXITCODE -eq 0) {
            Write-Host ""
            Write-Host "✅ Migration completed in $($duration.TotalSeconds) seconds!" -ForegroundColor Green
            Write-Host ""
            Write-Host "📁 Data stored in: $testDir\velesdb_data" -ForegroundColor Gray
            
            # Show file sizes
            $dataPath = Join-Path $testDir "velesdb_data"
            if (Test-Path $dataPath) {
                $size = (Get-ChildItem $dataPath -Recurse | Measure-Object -Property Length -Sum).Sum
                $sizeMB = [math]::Round($size / 1MB, 2)
                Write-Host "💾 Total size: $sizeMB MB" -ForegroundColor Gray
            }
        } else {
            Write-Host "❌ Migration failed!" -ForegroundColor Red
        }
    } else {
        Write-Host "⏭️ Skipping actual migration" -ForegroundColor Gray
    }
    
    Write-Host ""
    Write-Host "🧹 Test directory: $testDir" -ForegroundColor Gray
    Write-Host "   (delete manually when done testing)" -ForegroundColor DarkGray
}

Write-Host ""
Write-Host "╔══════════════════════════════════════════════════════════════╗" -ForegroundColor Green
Write-Host "║                    Testing Complete! ✅                        ║" -ForegroundColor Green
Write-Host "╚══════════════════════════════════════════════════════════════╝" -ForegroundColor Green
