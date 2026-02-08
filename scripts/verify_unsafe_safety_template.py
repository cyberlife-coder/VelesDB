#!/usr/bin/env python3
"""
SAFETY Template Verifier for Rust unsafe blocks.

Scans Rust source files for `unsafe {}` and `unsafe impl` blocks,
validating that each has an adjacent AGENTS-template SAFETY comment:
- SAFETY header line
- One or more condition bullet lines  
- Reason line

Exit codes:
- 0: All unsafe blocks have complete SAFETY documentation
- 1: One or more unsafe blocks missing required SAFETY template fields

Usage:
    python verify_unsafe_safety_template.py --files file1.rs file2.rs --strict
    python verify_unsafe_safety_template.py --inventory inventory.md --strict
"""

import argparse
import re
import sys
from pathlib import Path
from typing import List, Tuple, Optional


# Pattern to match unsafe blocks: unsafe { ... } or unsafe impl ...
UNSAFE_BLOCK_PATTERN = re.compile(
    r'\bunsafe\s*(?:\{|impl)',
    re.MULTILINE
)

# Pattern to match SAFETY comment header
SAFETY_HEADER_PATTERN = re.compile(
    r'//\s*SAFETY\s*:',
    re.IGNORECASE
)

# Pattern to match condition bullets (lines starting with // - or // * or similar)
CONDITION_BULLET_PATTERN = re.compile(
    r'//\s*(?:-\s+|\*\s+|\d+\.\s+)\w+',
    re.IGNORECASE
)

# Pattern to match Reason line
REASON_PATTERN = re.compile(
    r'//\s*Reason\s*:',
    re.IGNORECASE
)

# Lines to skip when looking for SAFETY comments (blank lines, attributes, etc.)
SKIP_LINE_PATTERN = re.compile(
    r'^\s*(?://\s*)?(?:#\[|//!|///|\s*$)',
    re.MULTILINE
)


def find_unsafe_sites(content: str) -> List[Tuple[int, str]]:
    """Find all unsafe block/impl sites with their line numbers."""
    sites = []
    lines = content.split('\n')
    
    for match in UNSAFE_BLOCK_PATTERN.finditer(content):
        # Calculate line number
        line_num = content[:match.start()].count('\n') + 1
        # Get the line content for context
        line_content = lines[line_num - 1] if line_num <= len(lines) else ""
        sites.append((line_num, line_content.strip()))
    
    return sites


def get_preceding_lines(content: str, line_num: int, max_lines: int = 20) -> List[str]:
    """Get up to max_lines preceding the given line number."""
    lines = content.split('\n')
    start = max(0, line_num - max_lines - 1)
    return lines[start:line_num - 1]


def check_safety_template(lines: List[str]) -> Tuple[bool, List[str]]:
    """
    Check if the preceding lines contain a complete SAFETY template.
    
    Returns:
        (is_valid, missing_fields)
        - is_valid: True if all required fields are present
        - missing_fields: List of missing field names
    """
    # Join lines for easier pattern matching
    text = '\n'.join(lines)
    
    has_header = SAFETY_HEADER_PATTERN.search(text) is not None
    has_condition = CONDITION_BULLET_PATTERN.search(text) is not None
    has_reason = REASON_PATTERN.search(text) is not None
    
    missing = []
    if not has_header:
        missing.append("SAFETY header")
    if not has_condition:
        missing.append("condition bullet(s)")
    if not has_reason:
        missing.append("Reason line")
    
    return len(missing) == 0, missing


def verify_file(filepath: Path, strict: bool = False) -> List[Tuple[int, str, List[str]]]:
    """
    Verify a single Rust file for SAFETY template completeness.
    
    Returns:
        List of (line_num, line_content, missing_fields) for non-compliant unsafe blocks
    """
    violations = []
    
    try:
        content = filepath.read_text(encoding='utf-8')
    except Exception as e:
        print(f"Error reading {filepath}: {e}", file=sys.stderr)
        return [(0, f"<error: {e}>", ["file read error"])]
    
    unsafe_sites = find_unsafe_sites(content)
    
    for line_num, line_content in unsafe_sites:
        preceding = get_preceding_lines(content, line_num)
        is_valid, missing = check_safety_template(preceding)
        
        if not is_valid:
            violations.append((line_num, line_content, missing))
    
    return violations


def parse_inventory(inventory_path: Path) -> List[Path]:
    """Parse inventory.md to extract list of in-scope Rust files."""
    files = []
    
    try:
        content = inventory_path.read_text(encoding='utf-8')
    except Exception as e:
        print(f"Error reading inventory {inventory_path}: {e}", file=sys.stderr)
        return []
    
    # Look for file: entries in inventory
    file_pattern = re.compile(r'^file:\s+(crates/velesdb-core/src/\S+\.rs)', re.MULTILINE)
    
    for match in file_pattern.finditer(content):
        file_path = match.group(1)
        full_path = Path(file_path)
        if full_path.exists():
            files.append(full_path)
        else:
            # Try relative to project root
            alt_path = Path('.') / file_path
            if alt_path.exists():
                files.append(alt_path)
    
    return files


def main():
    parser = argparse.ArgumentParser(
        description='Verify SAFETY comment templates for unsafe Rust blocks'
    )
    parser.add_argument(
        '--files',
        nargs='+',
        help='Rust files to verify'
    )
    parser.add_argument(
        '--inventory',
        type=Path,
        help='Path to inventory.md containing list of files to verify'
    )
    parser.add_argument(
        '--strict',
        action='store_true',
        help='Require complete SAFETY template (header + conditions + reason)'
    )
    parser.add_argument(
        '--verbose',
        '-v',
        action='store_true',
        help='Print detailed output for each file'
    )
    
    args = parser.parse_args()
    
    # Collect files to verify
    files_to_verify = []
    
    if args.files:
        files_to_verify.extend(Path(f) for f in args.files)
    
    if args.inventory:
        inventory_files = parse_inventory(args.inventory)
        files_to_verify.extend(inventory_files)
    
    if not files_to_verify:
        print("Error: No files specified. Use --files or --inventory.", file=sys.stderr)
        sys.exit(1)
    
    # Remove duplicates while preserving order
    seen = set()
    files_to_verify = [f for f in files_to_verify if not (f in seen or seen.add(f))]
    
    # Verify each file
    total_violations = 0
    files_with_violations = 0
    
    for filepath in files_to_verify:
        if not filepath.exists():
            print(f"Warning: File not found: {filepath}", file=sys.stderr)
            continue
        
        violations = verify_file(filepath, strict=args.strict)
        
        if violations:
            files_with_violations += 1
            total_violations += len(violations)
            
            print(f"\n{filepath}")
            for line_num, line_content, missing in violations:
                missing_str = ', '.join(missing)
                print(f"  {line_num}: {line_content[:60]}")
                print(f"    Missing: {missing_str}")
        elif args.verbose:
            print(f"\n{filepath}: OK (all unsafe blocks have SAFETY templates)")
    
    # Summary
    print(f"\n{'='*60}")
    print(f"Files checked: {len(files_to_verify)}")
    print(f"Files with violations: {files_with_violations}")
    print(f"Total violations: {total_violations}")
    
    if total_violations > 0:
        print(f"\nFAILED: {total_violations} unsafe block(s) missing required SAFETY template fields")
        sys.exit(1)
    else:
        print("\nPASSED: All unsafe blocks have complete SAFETY documentation")
        sys.exit(0)


if __name__ == '__main__':
    main()
