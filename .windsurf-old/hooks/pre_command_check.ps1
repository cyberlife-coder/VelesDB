#!/usr/bin/env pwsh
# VelesDB Pre-Command Hook - Block Dangerous Commands
# Can BLOCK the command by exiting with code 2

param()

$ErrorActionPreference = "Continue"

# Read JSON from stdin
$jsonInput = $input | Out-String
$data = $jsonInput | ConvertFrom-Json -ErrorAction SilentlyContinue

if (-not $data) {
    exit 0
}

$commandLine = $data.tool_info.command_line
$cwd = $data.tool_info.cwd

# Dangerous commands to block
$dangerousPatterns = @(
    'rm\s+-rf\s+/',           # rm -rf /
    'del\s+/s\s+/q\s+C:',     # Windows recursive delete root
    'format\s+C:',             # Format drive
    'git\s+push.*--force',     # Force push (warn only)
    'cargo\s+publish',         # Publish to crates.io (confirm)
    'npm\s+publish',           # Publish to npm (confirm)
    'DROP\s+DATABASE',         # SQL drop database
    'DROP\s+TABLE',            # SQL drop table
    'TRUNCATE\s+TABLE'         # SQL truncate
)

$warnPatterns = @(
    'git\s+reset\s+--hard',    # Git reset hard
    'git\s+clean\s+-fd',       # Git clean force
    'cargo\s+clean'            # Cargo clean (can take time to rebuild)
)

foreach ($pattern in $dangerousPatterns) {
    if ($commandLine -match $pattern) {
        Write-Host "`n🔴 BLOCKED: Dangerous command detected!" -ForegroundColor Red
        Write-Host "   Command: $commandLine" -ForegroundColor Red
        Write-Host "   Pattern: $pattern" -ForegroundColor DarkGray
        Write-Host "   Run manually if you're sure.`n" -ForegroundColor Yellow
        exit 2  # BLOCK
    }
}

foreach ($pattern in $warnPatterns) {
    if ($commandLine -match $pattern) {
        Write-Host "⚠️  WARNING: Potentially destructive command" -ForegroundColor Yellow
        Write-Host "   Command: $commandLine" -ForegroundColor White
        # Don't block, just warn
    }
}

exit 0
