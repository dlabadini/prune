# Prune

**The guardrail for AI-assisted refactoring.**

Prune detects structural and semantic anti-patterns in codebases—particularly those introduced by AI coding assistants—and provides structured refactor plans for cleanup.

## The Problem

AI coding tools (Copilot, Cursor, Claude, etc.) produce functional but messy code:
- Redundant logic (same concept implemented multiple ways)
- Interface proliferation (inline types everywhere)
- Over-abstraction (unnecessary indirection)
- Inconsistent patterns across files

Existing linters catch syntax issues, not these conceptual problems. **Prune fills this gap.**

## Installation

### Quick Install (macOS/Linux)

```bash
curl -fsSL https://prunecode.ai/install.sh | bash
```

### Homebrew (macOS)

```bash
brew install prunecode/tap/prune
```

### Cargo (from source)

```bash
cargo install prune
```

### Direct Download

Download binaries from [Releases](https://github.com/prunecode/prune/releases).

## Quick Start

```bash
# Launch interactive TUI in current directory
prune

# Or run a CLI scan
prune scan

# JSON output for CI
prune scan --json
```

## Usage

### Interactive TUI (default)

Just run `prune` in your project directory:

```
┌─ Prune ─────────────────────────────────────────────────┐
│ 895 files | 8300 symbols | 42 issues                   │
├────────────────────────────────────────────────────────┤
│ !!!! God File: utils.ts                                │
│ !!   Redundant Logic: getUserById and fetchUser        │
│ !!   Interface Sprawl: 4 types represent 'User'        │
│ !    Barrel Bloat: index.ts                            │
│ .    Dead Exports: 12 unused in helpers.ts             │
└────────────────────────────────────────────────────────┘
 j/k:nav  Enter:detail  c:copy  p:save  w:watch  ?:help
```

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `j/k` | Navigate up/down |
| `Enter` | View finding detail |
| `c` | Copy refactor plan to clipboard |
| `p` | Save plan to REFACTOR_PLAN.md |
| `w` | Toggle watch mode |
| `s` | Cycle strictness level |
| `f` | Filter by pattern type |
| `r` | Rescan codebase |
| `?` | Show help |
| `q` | Quit |

### CLI Commands

```bash
# Scan current directory
prune scan

# Scan specific path
prune scan -p /path/to/project

# JSON output
prune scan --json

# Generate refactor plan for a finding
prune plan <finding-id>

# Copy plan to clipboard
prune plan <finding-id> --copy

# Save plan to file
prune plan <finding-id> -o plan.md

# Initialize config
prune init

# Show current config
prune config
```

## Pattern Detection

Prune detects 6 anti-patterns:

| Pattern | Description |
|---------|-------------|
| **God File** | Files with too many exports or lines |
| **Interface Sprawl** | Multiple types representing the same concept |
| **Redundant Logic** | Similar functions that should be consolidated |
| **Barrel Bloat** | Index files re-exporting too many modules |
| **Dead Exports** | Exported symbols with no internal consumers |
| **Over-Abstraction** | Unnecessary wrapper functions or single-impl interfaces |

## Configuration

Create `.prunerc` in your project root:

```toml
# Strictness: relaxed | default | strict
strictness = "default"

# Files to exclude (in addition to .gitignore)
exclude = [
    "node_modules/**",
    "dist/**",
    "**/*.test.ts",
]

# Patterns to disable
disabled_patterns = []

# Custom thresholds
[thresholds]
god_file_max_exports = 25
god_file_max_lines = 600
interface_sprawl_similarity = 0.85
```

## Refactor Plans

When you select a finding, Prune generates a structured markdown plan:

```markdown
# Prune Refactor Plan

**Type:** Interface Sprawl
**Severity:** High
**Confidence:** 92%

## Issue
Multiple inline type definitions represent the same "User" domain concept.

## Locations
| File | Lines | Context |
|------|-------|---------|
| src/api/types.ts | 14-18 | User |
| src/components/UserCard.tsx | 8-12 | UserProps |

## Suggested Approach
1. Create canonical `User` interface in `src/types/user.ts`
2. Replace inline types with imports
3. Remove unused type definitions

## Acceptance Criteria
- [ ] Single `User` type definition
- [ ] No inline user-shaped types
- [ ] All existing functionality preserved
```

Copy this plan and feed it to your AI agent (Cursor, Claude Code, Aider, etc.) to execute the refactor.

## Pricing

| Tier | Price | Features |
|------|-------|----------|
| **Trial** | Free (14 days) | Full functionality |
| **Solo** | $99 one-time | CLI, all patterns, 1 year updates |
| **Pro** | $149/year | Desktop app, watch mode, ongoing updates |
| **Team** | $99/seat/year | Shared config, CI integration, priority support |

[Start Free Trial →](https://prunecode.ai/trial)

## License

Prune is source-available software. See [LICENSE](LICENSE) for details.

## Links

- [Website](https://prunecode.ai)
- [Documentation](https://prunecode.ai/docs)
- [Discord](https://discord.gg/prune)
- [Twitter](https://twitter.com/prunedev)
