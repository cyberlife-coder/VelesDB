<# 
.SYNOPSIS
    Compare benchmark results between x86_64 and ARM64 architectures.

.DESCRIPTION
    This script parses Criterion benchmark results from both architectures
    and generates a comparison table highlighting performance differences.

.PARAMETER x86ResultsPath
    Path to x86_64 benchmark results directory (Criterion output)

.PARAMETER arm64ResultsPath
    Path to ARM64 benchmark results directory (Criterion output)

.PARAMETER OutputFormat
    Output format: 'table', 'json', or 'markdown' (default: 'markdown')

.EXAMPLE
    .\compare-arch-benchmarks.ps1 -x86ResultsPath ".\x86-results" -arm64ResultsPath ".\arm64-results"

.NOTES
    EPIC-054/US-003: ARM64 Benchmark Harness
#>

param(
    [Parameter(Mandatory=$false)]
    [string]$x86ResultsPath = ".\benchmark-results\x86_64",
    
    [Parameter(Mandatory=$false)]
    [string]$arm64ResultsPath = ".\benchmark-results\arm64",
    
    [Parameter(Mandatory=$false)]
    [ValidateSet('table', 'json', 'markdown')]
    [string]$OutputFormat = 'markdown',
    
    [Parameter(Mandatory=$false)]
    [string]$OutputFile = ""
)

function Parse-CriterionResults {
    param([string]$ResultsPath)
    
    $results = @{}
    
    if (-not (Test-Path $ResultsPath)) {
        Write-Warning "Results path not found: $ResultsPath"
        return $results
    }
    
    # Parse estimates.json files from Criterion output
    Get-ChildItem -Path $ResultsPath -Recurse -Filter "estimates.json" | ForEach-Object {
        $benchName = $_.Directory.Parent.Name
        $content = Get-Content $_.FullName | ConvertFrom-Json
        
        if ($content.mean) {
            $results[$benchName] = @{
                Mean = $content.mean.point_estimate
                StdDev = $content.std_dev.point_estimate
                Median = $content.median.point_estimate
            }
        }
    }
    
    # Fallback: parse raw text output
    $rawFile = Join-Path $ResultsPath "raw.txt"
    if ((Test-Path $rawFile) -and $results.Count -eq 0) {
        $content = Get-Content $rawFile -Raw
        $pattern = '(?<name>\w+)\s+time:\s+\[(?<low>[\d.]+)\s+\w+\s+(?<mean>[\d.]+)\s+\w+\s+(?<high>[\d.]+)'
        
        [regex]::Matches($content, $pattern) | ForEach-Object {
            $name = $_.Groups['name'].Value
            $mean = [double]$_.Groups['mean'].Value
            $results[$name] = @{
                Mean = $mean
                StdDev = 0
                Median = $mean
            }
        }
    }
    
    return $results
}

function Compare-Results {
    param(
        [hashtable]$x86Results,
        [hashtable]$arm64Results
    )
    
    $comparison = @()
    $allBenchmarks = ($x86Results.Keys + $arm64Results.Keys) | Sort-Object -Unique
    
    foreach ($bench in $allBenchmarks) {
        $x86 = $x86Results[$bench]
        $arm64 = $arm64Results[$bench]
        
        $entry = [PSCustomObject]@{
            Benchmark = $bench
            x86_64_ns = if ($x86) { [math]::Round($x86.Mean, 2) } else { "N/A" }
            ARM64_ns = if ($arm64) { [math]::Round($arm64.Mean, 2) } else { "N/A" }
            Ratio = "N/A"
            Delta = "N/A"
            Status = "MISSING"
        }
        
        if ($x86 -and $arm64 -and $x86.Mean -gt 0) {
            $ratio = $arm64.Mean / $x86.Mean
            $delta = (($arm64.Mean - $x86.Mean) / $x86.Mean) * 100
            
            $entry.Ratio = [math]::Round($ratio, 3)
            $entry.Delta = "$([math]::Round($delta, 1))%"
            
            $entry.Status = switch ($ratio) {
                { $_ -lt 0.9 } { "🟢 ARM64 FASTER" }
                { $_ -gt 1.1 } { "🔴 x86 FASTER" }
                default { "🟡 PARITY" }
            }
        }
        
        $comparison += $entry
    }
    
    return $comparison
}

function Format-Markdown {
    param([array]$Comparison)
    
    $output = @"
# Architecture Benchmark Comparison

| Benchmark | x86_64 (ns) | ARM64 (ns) | Ratio | Delta | Status |
|-----------|-------------|------------|-------|-------|--------|
"@
    
    foreach ($row in $Comparison) {
        $output += "`n| $($row.Benchmark) | $($row.x86_64_ns) | $($row.ARM64_ns) | $($row.Ratio) | $($row.Delta) | $($row.Status) |"
    }
    
    $output += @"

## Legend

- **Ratio**: ARM64 time / x86_64 time (< 1.0 means ARM64 is faster)
- **🟢 ARM64 FASTER**: ARM64 is >10% faster
- **🔴 x86 FASTER**: x86_64 is >10% faster  
- **🟡 PARITY**: Within ±10%

## Summary

"@
    
    $faster = ($Comparison | Where-Object { $_.Status -like "*ARM64 FASTER*" }).Count
    $slower = ($Comparison | Where-Object { $_.Status -like "*x86 FASTER*" }).Count
    $parity = ($Comparison | Where-Object { $_.Status -like "*PARITY*" }).Count
    $missing = ($Comparison | Where-Object { $_.Status -eq "MISSING" }).Count
    
    $output += "- ARM64 faster: $faster benchmarks`n"
    $output += "- x86_64 faster: $slower benchmarks`n"
    $output += "- Parity: $parity benchmarks`n"
    if ($missing -gt 0) {
        $output += "- Missing data: $missing benchmarks`n"
    }
    
    return $output
}

# Main execution
Write-Host "Parsing x86_64 results from: $x86ResultsPath" -ForegroundColor Cyan
$x86Results = Parse-CriterionResults -ResultsPath $x86ResultsPath

Write-Host "Parsing ARM64 results from: $arm64ResultsPath" -ForegroundColor Cyan
$arm64Results = Parse-CriterionResults -ResultsPath $arm64ResultsPath

Write-Host "Comparing results..." -ForegroundColor Cyan
$comparison = Compare-Results -x86Results $x86Results -arm64Results $arm64Results

switch ($OutputFormat) {
    'table' {
        $comparison | Format-Table -AutoSize
    }
    'json' {
        $comparison | ConvertTo-Json -Depth 3
    }
    'markdown' {
        $markdown = Format-Markdown -Comparison $comparison
        if ($OutputFile) {
            $markdown | Out-File -FilePath $OutputFile -Encoding UTF8
            Write-Host "Output written to: $OutputFile" -ForegroundColor Green
        } else {
            $markdown
        }
    }
}

if ($comparison.Count -eq 0) {
    Write-Warning "No benchmark results found. Ensure Criterion output exists."
    exit 1
}

Write-Host "`n✅ Comparison complete" -ForegroundColor Green
