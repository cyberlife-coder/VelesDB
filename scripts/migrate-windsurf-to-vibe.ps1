<#
.SYNOPSIS
    Migrates .windsurf/ skills and workflows to .vibe/ format for Mistral Vibe.

.DESCRIPTION
    Converts:
    - Skills (.windsurf/skills/*/SKILL.md) to .vibe/skills/*/SKILL.md
    - Workflows (.windsurf/workflows/*.md) to .vibe/skills/*/SKILL.md (user-invocable)
    - Creates config.toml with MCP servers
    - Creates custom agents in ~/.vibe/agents/
    - Creates system prompt in ~/.vibe/prompts/

.NOTES
    Re-runnable: overwrites existing .vibe/ files safely.
    Run from project root: .\scripts\migrate-windsurf-to-vibe.ps1
#>

param(
    [string]$ProjectRoot = (Get-Location).Path,
    [switch]$DryRun,
    [switch]$SkipGlobal
)

$ErrorActionPreference = "Stop"

# --- Paths ---
$WindsurfDir = Join-Path $ProjectRoot ".windsurf"
$VibeDir     = Join-Path $ProjectRoot ".vibe"
$VibeHome    = Join-Path $env:USERPROFILE ".vibe"

if (-not (Test-Path $WindsurfDir)) {
    Write-Error "ERROR: .windsurf/ directory not found at $WindsurfDir"
    exit 1
}

Write-Host "=== Windsurf to Mistral Vibe Migration ===" -ForegroundColor Cyan
Write-Host "  Source:  $WindsurfDir" -ForegroundColor DarkGray
Write-Host "  Target:  $VibeDir" -ForegroundColor DarkGray
Write-Host "  Global:  $VibeHome" -ForegroundColor DarkGray
Write-Host ""

# --- Helper: Parse YAML frontmatter from markdown ---
function Parse-Frontmatter {
    param([string]$Content)

    $result = @{
        Frontmatter = @{}
        Body = ""
    }

    if ($Content -match "^---\s*\r?\n([\s\S]*?)\r?\n---\s*\r?\n([\s\S]*)$") {
        $yamlBlock = $Matches[1]
        $result.Body = $Matches[2]

        foreach ($line in $yamlBlock -split "`n") {
            $line = $line.Trim()
            if ($line -match "^(\w[\w-]*):\s*(.+)$") {
                $result.Frontmatter[$Matches[1]] = $Matches[2].Trim()
            }
        }
    } else {
        $result.Body = $Content
    }

    return $result
}

# --- Helper: Determine allowed-tools based on content ---
function Get-AllowedTools {
    param([string]$Body, [bool]$IsWorkflow)

    $tools = [System.Collections.Generic.List[string]]::new()
    $tools.Add("read_file")
    $tools.Add("grep")

    if ($Body -match "git |cargo |npm |bash|powershell|command") {
        $tools.Add("bash")
    }
    if ($Body -match "Create|Modify|Write|commit|edit") {
        $tools.Add("write_file")
        $tools.Add("search_replace")
    }
    if ($Body -match "task|subagent|delegate") {
        $tools.Add("task")
    }
    if ($Body -match "ask_user|question|clarif") {
        $tools.Add("ask_user_question")
    }
    if ($Body -match "todo|todo_list") {
        $tools.Add("todo")
    }

    if ($IsWorkflow -and -not $tools.Contains("bash")) {
        $tools.Add("bash")
    }
    if ($IsWorkflow -and -not $tools.Contains("write_file")) {
        $tools.Add("write_file")
        $tools.Add("search_replace")
    }

    return ($tools | Sort-Object -Unique)
}

# --- Helper: Build Vibe SKILL.md content ---
function Build-VibeSkill {
    param(
        [string]$Name,
        [string]$Description,
        [string]$Body,
        [bool]$UserInvocable,
        [string[]]$AllowedTools
    )

    $toolsYaml = ($AllowedTools | ForEach-Object { "  - $_" }) -join "`n"
    $invocable = if ($UserInvocable) { "true" } else { "false" }

    $lines = @(
        "---"
        "name: $Name"
        "description: $Description"
        "license: Proprietary"
        "user-invocable: $invocable"
        "allowed-tools:"
        $toolsYaml
        "---"
        ""
    )
    $frontmatter = $lines -join "`n"

    return "$frontmatter$Body"
}

# --- Helper: Write file with directory creation ---
function Write-MigrationFile {
    param([string]$Path, [string]$Content)

    if (-not $DryRun) {
        $dir = Split-Path $Path -Parent
        New-Item -ItemType Directory -Path $dir -Force | Out-Null
        [System.IO.File]::WriteAllText($Path, $Content, [System.Text.UTF8Encoding]::new($false))
    }
}

# =============================================
# Step 1: Create .vibe/ directory structure
# =============================================
Write-Host "[Step 1] Creating .vibe/ directory structure..." -ForegroundColor Yellow

if (-not $DryRun) {
    New-Item -ItemType Directory -Path (Join-Path $VibeDir "skills") -Force | Out-Null
}
Write-Host "    OK: $(Join-Path $VibeDir 'skills')" -ForegroundColor DarkGray

# =============================================
# Step 2: Migrate Skills (7 files)
# =============================================
Write-Host "[Step 2] Migrating skills..." -ForegroundColor Yellow

$skillsDir = Join-Path $WindsurfDir "skills"
$skillCount = 0

if (Test-Path $skillsDir) {
    Get-ChildItem -Path $skillsDir -Directory | ForEach-Object {
        $skillName = $_.Name
        $sourceFile = Join-Path $_.FullName "SKILL.md"

        if (Test-Path $sourceFile) {
            $content = Get-Content $sourceFile -Raw -Encoding UTF8
            $parsed = Parse-Frontmatter -Content $content

            $name = if ($parsed.Frontmatter["name"]) { $parsed.Frontmatter["name"] } else { $skillName }
            $desc = if ($parsed.Frontmatter["description"]) { $parsed.Frontmatter["description"] } else { "Skill: $skillName" }
            $tools = Get-AllowedTools -Body $parsed.Body -IsWorkflow $false

            $vibeContent = Build-VibeSkill `
                -Name $name `
                -Description $desc `
                -Body $parsed.Body `
                -UserInvocable $false `
                -AllowedTools $tools

            $targetFile = Join-Path $VibeDir "skills" $skillName "SKILL.md"
            Write-MigrationFile -Path $targetFile -Content $vibeContent

            $skillCount++
            Write-Host "    [OK] Skill: $skillName" -ForegroundColor Green
        }
    }
}

Write-Host "    Total skills migrated: $skillCount" -ForegroundColor Cyan

# =============================================
# Step 3: Convert Workflows to Skills (30 files)
# =============================================
Write-Host "[Step 3] Converting workflows to invocable skills..." -ForegroundColor Yellow

$workflowsDir = Join-Path $WindsurfDir "workflows"
$workflowCount = 0

if (Test-Path $workflowsDir) {
    Get-ChildItem -Path $workflowsDir -Filter "*.md" | ForEach-Object {
        $skillName = $_.BaseName

        $content = Get-Content $_.FullName -Raw -Encoding UTF8
        $parsed = Parse-Frontmatter -Content $content

        $desc = if ($parsed.Frontmatter["description"]) {
            $parsed.Frontmatter["description"]
        } else {
            "Workflow: $skillName"
        }

        $tools = Get-AllowedTools -Body $parsed.Body -IsWorkflow $true

        $vibeContent = Build-VibeSkill `
            -Name $skillName `
            -Description $desc `
            -Body $parsed.Body `
            -UserInvocable $true `
            -AllowedTools $tools

        $targetFile = Join-Path $VibeDir "skills" $skillName "SKILL.md"
        Write-MigrationFile -Path $targetFile -Content $vibeContent

        $workflowCount++
        Write-Host "    [OK] Workflow -> Skill: $skillName (user-invocable)" -ForegroundColor Green
    }
}

Write-Host "    Total workflows converted: $workflowCount" -ForegroundColor Cyan

# =============================================
# Step 4: Create .vibe/config.toml (local)
# =============================================
Write-Host "[Step 4] Creating .vibe/config.toml..." -ForegroundColor Yellow

$configLines = @(
    '# VelesDB - Mistral Vibe Local Configuration'
    '# Docs: https://docs.mistral.ai/mistral-vibe/introduction/configuration'
    ''
    '# Use VelesDB-specific system prompt'
    'system_prompt_id = "velesdb"'
    ''
    '# --- MCP Servers ---'
    ''
    '# Brave Search - Web search and local search'
    '[[mcp_servers]]'
    'name = "brave"'
    'transport = "stdio"'
    'command = "npx"'
    'args = ["-y", "@anthropic-ai/mcp-server-brave-search"]'
    'env = { "BRAVE_API_KEY" = "" }'
    'startup_timeout_sec = 15'
    'tool_timeout_sec = 30'
    ''
    '# Context7 - Documentation and code references'
    '[[mcp_servers]]'
    'name = "context7"'
    'transport = "stdio"'
    'command = "npx"'
    'args = ["-y", "@upstash/context7-mcp"]'
    'startup_timeout_sec = 15'
    'tool_timeout_sec = 60'
    ''
    '# Sequential Thinking'
    '[[mcp_servers]]'
    'name = "sequential-thinking"'
    'transport = "stdio"'
    'command = "npx"'
    'args = ["-y", "@anthropic-ai/mcp-server-sequential-thinking"]'
    'startup_timeout_sec = 10'
    'tool_timeout_sec = 120'
    ''
    '# --- Tool Permissions ---'
    ''
    '[tools.read_file]'
    'permission = "always"'
    ''
    '[tools.grep]'
    'permission = "always"'
    ''
    '[tools.bash]'
    'permission = "ask"'
    ''
    '[tools.write_file]'
    'permission = "ask"'
    ''
    '[tools.search_replace]'
    'permission = "ask"'
)
$configToml = $configLines -join "`n"

$configPath = Join-Path $VibeDir "config.toml"
Write-MigrationFile -Path $configPath -Content $configToml
Write-Host "    [OK] Created: $configPath" -ForegroundColor Green

# =============================================
# Step 5: Create global ~/.vibe/ config
# =============================================
if (-not $SkipGlobal) {
    Write-Host "[Step 5] Creating global ~/.vibe/ configuration..." -ForegroundColor Yellow

    # Create directories
    @("agents", "prompts", "skills") | ForEach-Object {
        $dir = Join-Path $VibeHome $_
        if (-not $DryRun) {
            New-Item -ItemType Directory -Path $dir -Force | Out-Null
        }
    }

    # -- Agent: VelesDB Dev --
    $devAgentLines = @(
        '# VelesDB Development Agent'
        '# Usage: vibe --agent velesdb-dev'
        ''
        'display_name = "VelesDB Dev"'
        'description = "Development agent for VelesDB Rust codebase. TDD, atomic commits, Rust best practices."'
        'safety = "neutral"'
        'auto_approve = false'
        'system_prompt_id = "velesdb"'
    )
    Write-MigrationFile -Path (Join-Path $VibeHome "agents" "velesdb-dev.toml") -Content ($devAgentLines -join "`n")
    Write-Host "    [OK] Agent: velesdb-dev.toml" -ForegroundColor Green

    # -- Agent: VelesDB Auto --
    $autoAgentLines = @(
        '# VelesDB Auto-Approve Agent (for trusted operations)'
        '# Usage: vibe --agent velesdb-auto'
        ''
        'display_name = "VelesDB Auto"'
        'description = "Auto-approve agent for VelesDB. Use for repetitive tasks like formatting, refactoring."'
        'safety = "destructive"'
        'auto_approve = true'
        'system_prompt_id = "velesdb"'
    )
    Write-MigrationFile -Path (Join-Path $VibeHome "agents" "velesdb-auto.toml") -Content ($autoAgentLines -join "`n")
    Write-Host "    [OK] Agent: velesdb-auto.toml" -ForegroundColor Green

    # -- Agent: VelesDB Review (subagent) --
    $reviewAgentLines = @(
        '# VelesDB Review Subagent (read-only)'
        '# Used as a subagent for code review tasks'
        ''
        'display_name = "VelesDB Review"'
        'description = "Read-only code review subagent for VelesDB. Analyzes code quality and suggests improvements."'
        'safety = "safe"'
        'agent_type = "subagent"'
        'enabled_tools = ["read_file", "grep"]'
    )
    Write-MigrationFile -Path (Join-Path $VibeHome "agents" "velesdb-review.toml") -Content ($reviewAgentLines -join "`n")
    Write-Host "    [OK] Agent: velesdb-review.toml (subagent)" -ForegroundColor Green

    # -- System Prompt: VelesDB --
    # Reason: Loaded from a separate file to avoid PowerShell parsing issues with markdown
    $promptPath = Join-Path $VibeHome "prompts" "velesdb.md"
    $promptTemplatePath = Join-Path $PSScriptRoot "vibe-velesdb-prompt.md"

    if (Test-Path $promptTemplatePath) {
        # Use template file if it exists next to this script
        $promptContent = Get-Content $promptTemplatePath -Raw -Encoding UTF8
        Write-MigrationFile -Path $promptPath -Content $promptContent
        Write-Host "    [OK] System prompt: velesdb.md (from template)" -ForegroundColor Green
    } else {
        # Generate a minimal prompt inline
        $promptLines = @(
            '# VelesDB Core - System Prompt'
            ''
            'You are an expert Rust developer working on VelesDB, a cognitive memory engine for AI agents.'
            ''
            '## Project Identity'
            ''
            'VelesDB Core = Local cognitive engine for AI agents.'
            'Vector + Graph + Symbolic in a single engine. Microsecond latency. Local-first (WASM, desktop, mobile, edge).'
            ''
            '## Mandatory Rules'
            ''
            '### TDD'
            '1. RED: Write test FIRST (separate file: *_tests.rs)'
            '2. GREEN: Implement MINIMUM to pass'
            '3. REFACTOR: Clean while keeping tests green'
            ''
            '### Rust Coding'
            '- No unwrap() on user data - use ? or expect("message")'
            '- unsafe requires // SAFETY: comment'
            '- No hardcoded secrets'
            '- Bounds checking on arrays/vectors'
            '- clone() justified by comment if hot-path'
            '- No println!/dbg!/eprintln! in production - use tracing'
            '- Cosine similarity: value.clamp(-1.0, 1.0)'
            '- Numeric casts: try_from() instead of as'
            '- Files under 300 lines - split into modules'
            ''
            '### Before EVERY Commit'
            '```'
            'cargo fmt --all'
            'cargo clippy -- -D warnings'
            'cargo deny check'
            'cargo test --workspace'
            '```'
            'If ANY fails, DO NOT commit.'
            ''
            '### Git Flow'
            'main (protected) -> develop -> feature/EPIC-XXX-US-YYY'
            'Commit: type(scope): description [EPIC-XXX/US-YYY]'
            ''
            '### Test Naming'
            'test_[function]_[scenario]_[expected_result]'
            'Tests in separate files (*_tests.rs), NOT inline.'
            ''
            '## Planning System'
            'File-based planning in .planning/:'
            '- PROJECT.md, ROADMAP.md, STATE.md'
            '- phases/*/PLAN.md, phases/*/SUMMARY.md'
            'Use /gsd-progress for status, /gsd-help for commands.'
        )
        Write-MigrationFile -Path $promptPath -Content ($promptLines -join "`n")
        Write-Host "    [OK] System prompt: velesdb.md (generated)" -ForegroundColor Green
    }

} else {
    Write-Host "[Step 5] Skipped (--SkipGlobal)" -ForegroundColor DarkGray
}

# =============================================
# Step 6: Update .gitignore
# =============================================
Write-Host "[Step 6] Checking .gitignore..." -ForegroundColor Yellow

$gitignorePath = Join-Path $ProjectRoot ".gitignore"
if (Test-Path $gitignorePath) {
    $gitignore = Get-Content $gitignorePath -Raw
    if ($gitignore -notmatch "\.vibe/") {
        if (-not $DryRun) {
            Add-Content -Path $gitignorePath -Value "`n# Mistral Vibe (local config)`n.vibe/"
        }
        Write-Host "    [OK] Added .vibe/ to .gitignore" -ForegroundColor Green
    } else {
        Write-Host "    [OK] .vibe/ already in .gitignore" -ForegroundColor DarkGray
    }
} else {
    Write-Host "    [WARN] No .gitignore found" -ForegroundColor DarkYellow
}

# =============================================
# Summary
# =============================================
Write-Host ""
Write-Host "=== Migration Complete! ===" -ForegroundColor Green
Write-Host "  Skills migrated:     $skillCount" -ForegroundColor White
Write-Host "  Workflows converted: $workflowCount" -ForegroundColor White
Write-Host "  Total slash commands: $($skillCount + $workflowCount)" -ForegroundColor White
Write-Host ""
Write-Host "  Local config:  $VibeDir" -ForegroundColor White
if (-not $SkipGlobal) {
    Write-Host "  Global config: $VibeHome" -ForegroundColor White
}
Write-Host ""
Write-Host "  Next steps:" -ForegroundColor Yellow
Write-Host "  1. Install Vibe:     pip install mistral-vibe" -ForegroundColor DarkGray
Write-Host "  2. Setup API key:    vibe --setup" -ForegroundColor DarkGray
Write-Host "  3. Configure MCP:    Edit .vibe/config.toml (add API keys)" -ForegroundColor DarkGray
Write-Host "  4. Test:             vibe --workdir $ProjectRoot" -ForegroundColor DarkGray
Write-Host "  5. Try a command:    /gsd-help" -ForegroundColor DarkGray
Write-Host ""

if ($DryRun) {
    Write-Host "  [WARN] DRY RUN - No files were created" -ForegroundColor DarkYellow
}
