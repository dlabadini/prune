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
cargo install prunecode
```

### Direct Download

Download binaries from [Releases](https://github.com/prunecode/prune/releases).

## Quick Start

```bash
# In the repo you want to check
prune init
prune scan
```

`prune init` writes `.prunerc` (commit this) and adds `.prune/` to `.gitignore`. The local index lives only under `.prune/`.

```bash
# Interactive TUI
prune

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

# Report only findings missing from the committed baseline; exit 1 if any (CI)
prune baseline                 # writes .prune-baseline.json
prune scan --baseline --ci

# Only findings in files changed since a ref (default HEAD) and the files importing them
prune scan --changed main

# Safe autofix (dry run unless --apply); each fix is kept only if the check passes
prune fix --apply --check "bun run typecheck"
prune rename src/thread.ts ThreadRow ThreadSummary --apply --check "bun run typecheck"

# Write .prunerc and ignore .prune/
prune init

# Show current config
prune config
```

## Pattern Detection

| Pattern | Description |
|---------|-------------|
| **God File** | Files whose behavior (functions, classes) exceeds the export limit, or whose distinct concepts do while the file is also long. A validator and its inferred type count once; validators, schemas and `as const` data are declarations, not behavior |
| **Interface Sprawl** | Types, interfaces and object validators (`v.object`, `z.object`) that restate the same fields |
| **Lexicon** | One shape under several names, and glossary terms used for unrelated shapes |
| **Redundant Logic** | Similar functions that should be consolidated |
| **Barrel Bloat** | Index files re-exporting too many modules |
| **Dead Exports** | Unreachable files and exports classified by reachability (below) |
| **Import Cycle** | Runtime value cycles; type-only cycles reported separately at low severity |
| **Over-Abstraction** | Unnecessary wrapper functions or single-impl interfaces |

### Dead code by reachability

Prune builds the module graph from entry points and walks it twice: from product entries and
from test files. Imports resolve through barrels (`export *`, aliased re-exports) and namespace
imports to the symbol that defines them. Each export gets one class, shown in findings and in
`symbols[].usage` in JSON:

| Class | Meaning | Fix |
|-------|---------|-----|
| `used` | Reached from product code, or consumed by a framework or tool | — |
| `test_only` | Only tests use it | Delete it with its tests |
| `same_file_only` | Only its own file uses it | Drop `export` (`prune fix`) |
| `unused` | Nothing reachable uses it | Delete it |
| `unreachable` | Its file is not reachable from any entry | Delete the file (`prune fix`) |

Entries are discovered from `package.json` (`main`, `module`, `types`, `exports`, `bin`, and file
paths in `scripts`), tool config files (`*.config.*`, paths named in root `bunfig.toml`, CI yaml,
`vercel.json`), files with a `#!` line, `.d.ts` declarations, and framework adapters:

- **Next.js** (any directory with `next.config.*`): `app/**/{page,layout,route,loading,error,not-found,template,default,opengraph-image,icon,...}`, `pages/**`, `public/**`, root `middleware`/`proxy`/`instrumentation`. Their exports are framework roots.
- **Convex** (`convex.json` `functions`, else `convex/`): every function module is an entry; `schema`, `http`, `crons`, `convex.config`, `auth.config` are roots. Registered functions (`query`, `internalMutation`, wrappers ending in `Query`/`Mutation`/`Action`) are used only when referenced: `api.a.b.fn` / `internal.a.b.fn` / `components.c.fn` member chains, `"a/b:fn"` strings (`makeFunctionReference`), or `convex run a/b:fn` in scripts, strings, or comments.

Exports of a `"private": true` workspace package entry count as used only when something imports them.
Test files are read as reference sources even when `exclude` matches them; they are never reported.

### Accuracy

`cargo test --test accuracy -- --nocapture` scans labeled fixtures (`tests/fixtures/*/labels.json`)
and prints precision and recall per pattern; it fails below 90%. On the fixtures every metric is
at 100%.

## Configuration

Create `.prunerc` in your project root, or run `prune init`. Generated data stays in `.prune/` and should not be committed.

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
dead_exports_min_per_file = 1   # report a file once it has this many dead exports

# Entry points beyond the discovered ones (every export counts as used)
[entries]
files = ["vercel.ts", "scripts/**"]
test_files = ["**/*.test.ts", "**/__tests__/**", "e2e/**"]   # read for references only

# Which dead-code classes become findings
[dead_exports]
report_unreachable_files = true
report_test_only = true
report_same_file = true

# Framework adapters (defaults shown; detected automatically)
[frameworks.convex]
dirs = ["convex"]
function_builders = ["query", "mutation", "action", "internalQuery", "internalMutation", "internalAction", "httpAction"]
builder_suffixes = ["Query", "Mutation", "Action"]

[frameworks.next]
roots = ["."]

# Boundaries: a fenced boundary owns its types. Duplicate shapes across its fence are
# intentional and not reported; duplicates inside it are. Findings carry their boundary.
[[boundaries]]
name = "inbox"
paths = ["packages/inbox/**"]
fenced = true

# Lexicon: flag one term used for unrelated shapes
[lexicon]
smash_names = false   # "packs N resource tokens" heuristic, off by default
[lexicon.glossary]
draft = "a held reply awaiting review"

# Check command for `prune fix` / `prune rename`; a failing fix is reverted
[fix]
check = "bun run typecheck"
```

Finding ids derive from the pattern and the symbols (or files) involved, not titles or line
numbers, so they stay stable across unrelated edits and work as baseline keys.

## MCP

`prune mcp` serves tools over stdio. Besides `scan`, `plan`, `verify`, `compose_plan`, and
`recent_files`, agents can ask before editing:

| Tool | Answers |
|------|---------|
| `who_uses` | Usage class of an export, every file that uses it (through barrels, namespaces, Convex references), and barrels that re-export it |
| `safe_to_delete` | Whether deleting or un-exporting it is safe, with reasons |
| `rename_impact` | Files and edit counts a rename touches (nothing is written) |

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
