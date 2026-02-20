# Prune: Product Requirements Document

**Version:** 1.0
**Date:** December 30, 2024
**Status:** Draft

---

## Executive Summary

Prune is a local-first developer tool that detects structural and semantic anti-patterns in codebases—particularly those introduced by AI coding assistants—and provides a feedback loop for AI-assisted refactoring. Prune doesn't fix code; it watches, scores, and guides while the user's AI agent of choice does the work.

**Core insight:** AI coding tools produce functional but messy code. Existing linters catch syntax issues, not conceptual problems like interface sprawl, redundant logic, or over-abstraction. Prune fills this gap.

**Core loop:**
1. Prune detects anti-patterns, calibrated to team context
2. User selects an issue to fix
3. Prune generates a structured refactor plan (markdown)
4. User feeds plan to any AI agent (Cursor, Aider, Claude Code, etc.)
5. Agent makes changes
6. Prune watches in real-time, validates fixes, surfaces new issues
7. Repeat until clean

---

## Problem Statement

### The AI Slop Problem
AI coding assistants (Copilot, Cursor, Claude, etc.) dramatically accelerate code generation but produce:
- Redundant logic (same concept implemented multiple ways)
- Interface proliferation (inline types everywhere instead of shared domain types)
- Over-abstraction (unnecessary indirection, premature generalization)
- Inconsistent patterns (different approaches to the same problem across files)

### Why Existing Tools Fail
- **Linters (ESLint, etc.):** Catch syntax and style issues, not semantic/structural problems
- **Static analysis (SonarQube, CodeClimate):** Focus on bugs, security, complexity metrics—not conceptual coherence
- **Clone detectors:** Find syntactic duplication, miss semantic similarity
- **AI agents themselves:** Don't know when to stop, can't self-evaluate quality

### The Gap
No tool currently:
- Detects *conceptual* anti-patterns (not just syntactic)
- Calibrates severity to team context and goals
- Provides real-time feedback during AI-assisted refactoring
- Stays local-first and works with any AI agent

---

## Solution Overview

### What Prune Is
- A local CLI and desktop app for detecting code anti-patterns
- A referee for AI-assisted refactoring (watches and scores, doesn't play)
- A context generator that makes AI agents more effective
- Goal-aware: patterns are scored against team size, codebase lifespan, philosophy

### What Prune Is Not
- An AI coding assistant (no code generation)
- A linter replacement (complements, doesn't replace ESLint/Prettier)
- A cloud service (all processing local, no data leaves machine)
- A CI/CD gate (possible later, not v1)

### Core Value Proposition
**"The guardrail for AI-assisted refactoring."**

You trust AI to write code. Prune tells you when that trust was misplaced, and guides cleanup without making things worse.

---

## User Personas

### Primary: Solo Dev / Small Team Lead
- Uses AI coding tools daily
- Notices codebase getting messier but can't articulate why
- Doesn't have time for manual code review of AI output
- Willing to pay for tools that save time
- Wants local/private—won't send code to another cloud service

### Secondary: Engineering Manager (Growth Team)
- Team scaling from 5 → 15
- AI adoption increasing, code consistency decreasing
- Needs visibility into codebase health trends
- Wants to establish standards without slowing team down

### Tertiary: Consultant / Contractor
- Joins projects with existing AI-generated code
- Needs to quickly assess codebase quality
- Uses report to scope cleanup work and set expectations

---

## Functional Requirements

### Phase 1: Core Detection (MVP)

#### F1.1: Codebase Indexing
- **F1.1.1:** Index TypeScript/JavaScript files using tree-sitter parsing
- **F1.1.2:** Extract symbols (functions, classes, interfaces, types, variables)
- **F1.1.3:** Build relationship graph (imports, exports, references)
- **F1.1.4:** Generate semantic embeddings per code block (function/class level)
- **F1.1.5:** Store index in local SQLite + vector store (lancedb)
- **F1.1.6:** Incremental re-indexing on file changes
- **F1.1.7:** Respect .gitignore and custom exclusion patterns

#### F1.2: Pattern Detection
- **F1.2.1:** Interface sprawl—detect multiple inline types representing same concept
- **F1.2.2:** Redundant logic—semantically similar code blocks (embedding distance)
- **F1.2.3:** God files—files with too many exports or excessive line count
- **F1.2.4:** Barrel bloat—index files re-exporting large portions of codebase
- **F1.2.5:** Dead exports—exported symbols with no internal consumers
- **F1.2.6:** Over-abstraction—layers of indirection with single implementations

#### F1.3: Goal Calibration (Optional)
- **F1.3.1:** Ship with opinionated defaults (small team, pragmatic, ship-fast)
- **F1.3.2:** Single config option: `strictness: relaxed | default | strict`
- **F1.3.3:** Per-pattern threshold overrides for power users (e.g., `interface_sprawl.threshold: 5`)
- **F1.3.4:** Persist in local config file (.keelrc or keel.config.json)
- **F1.3.5:** No onboarding wizard—works out of the box, users tune if they want

**Rationale:** Most users won't configure anything. Defaults should be useful for the 80% case (small-to-mid team, wants less mess). Power users can tune individual thresholds. Questionnaires are scope bloat.

#### F1.4: CLI Interface

**Primary: Interactive TUI (default)**
- **F1.4.1:** `keel` — launches interactive TUI in current directory
- **F1.4.2:** Initial scan runs automatically, shows findings list
- **F1.4.3:** Arrow keys to navigate findings, Enter to drill into detail
- **F1.4.4:** Keyboard shortcuts for common actions:
  - `c` — copy refactor plan to clipboard
  - `p` — generate and save REFACTOR_PLAN.md
  - `w` — toggle watch mode
  - `s` — cycle strictness (relaxed → default → strict)
  - `f` — filter findings by type
  - `r` — re-scan
  - `q` — quit
  - `?` — show help/shortcuts
- **F1.4.5:** Finding detail view shows: severity, locations, explanation, code snippets
- **F1.4.6:** Status bar: current strictness, watch mode status, last scan time

**Secondary: Direct Commands (for scripting/CI)**
- **F1.4.7:** `keel scan` — run scan, output to stdout, exit
- **F1.4.8:** `keel scan --json` — JSON output for tooling integration
- **F1.4.9:** `keel scan --watch` — headless watch mode (logs changes)
- **F1.4.10:** `keel plan <finding-id>` — output plan to stdout
- **F1.4.11:** `keel plan <finding-id> --copy` — copy plan to clipboard
- **F1.4.12:** `keel init` — generate .keelrc with defaults
- **F1.4.13:** `keel --help` — show all commands

**Design Principle:** Running `keel` with no args drops you into a guided workflow. Slash commands (`keel scan`, `keel plan`) are for automation and power users. The TUI is the primary interface for day-to-day use.

#### F1.5: Refactor Plan Generation
- **F1.5.1:** Generate structured markdown plan for selected finding
- **F1.5.2:** Include: issue description, severity, locations, suggested approach
- **F1.5.3:** Include: acceptance criteria (machine-verifiable where possible)
- **F1.5.4:** Include: relevant code snippets (collapsed/truncated for token efficiency)
- **F1.5.5:** Output to file or clipboard

### Phase 2: Live Monitoring & Desktop UI

#### F2.1: Watch Mode (Enhanced)
- **F2.1.1:** File watcher with debounced re-indexing
- **F2.1.2:** Real-time pattern re-evaluation on changes
- **F2.1.3:** Terminal UI showing pattern counts changing live
- **F2.1.4:** Alert when fix introduces new issue
- **F2.1.5:** Progress tracking: "3/7 instances fixed, 4 remaining"

#### F2.2: Desktop Application (Electron)
- **F2.2.1:** Dashboard showing all findings, sortable by severity/type
- **F2.2.2:** Codebase visualization (file tree with issue markers)
- **F2.2.3:** Finding detail view with code snippets, explanation
- **F2.2.4:** One-click plan generation
- **F2.2.5:** Settings panel (strictness level, pattern toggles)
- **F2.2.6:** Watch mode status and live updates

#### F2.3: Session Tracking
- **F2.3.1:** Track refactoring sessions (start, changes, outcomes)
- **F2.3.2:** Before/after comparison for completed fixes
- **F2.3.3:** Session history (what was fixed, when, improvement delta)

### Phase 3: Advanced Detection & Integrations

#### F3.1: Additional Patterns
- **F3.1.1:** Coupling analysis—files that always change together
- **F3.1.2:** Inconsistent naming—similar concepts with different names
- **F3.1.3:** API surface bloat—public exports that should be internal
- **F3.1.4:** Test coverage correlation—complex code with no tests
- **F3.1.5:** Import cycle detection

#### F3.2: Git Integration
- **F3.2.1:** Analyze git history for temporal coupling
- **F3.2.2:** Identify recently-introduced issues (blame integration)
- **F3.2.3:** Trend analysis—is codebase getting better or worse over time

#### F3.3: AI Agent Integration (Optional)
- **F3.3.1:** Direct integration with Claude API (user provides key)
- **F3.3.2:** Orchestrated fix: Keel sends plan, receives changes, validates
- **F3.3.3:** Aider/Cursor plugin or CLI handoff
- **F3.3.4:** MCP server mode for Claude Desktop integration

#### F3.4: Additional Languages
- **F3.4.1:** Python support
- **F3.4.2:** Go support
- **F3.4.3:** Rust support
- **F3.4.4:** Language-specific pattern variants

### Phase 4: Team & Expansion

#### F4.1: Team Features
- **F4.1.1:** Shared goal configuration
- **F4.1.2:** Export/import config for team consistency
- **F4.1.3:** CI integration—run Keel in pipeline, fail on threshold
- **F4.1.4:** Trend dashboard for team leads

#### F4.2: Corpus Expansion (Future)
- **F4.2.1:** Markdown/documentation analysis
- **F4.2.2:** API spec (OpenAPI) consistency checking
- **F4.2.3:** Design token analysis
- **F4.2.4:** Adapter architecture for new corpus types

---

## Technical Requirements

### T1: Core Architecture

#### T1.1: Rust Core
- **T1.1.1:** All indexing and pattern detection in Rust for performance
- **T1.1.2:** Target: 50k LOC TypeScript repo indexed in <30 seconds
- **T1.1.3:** Memory efficient: handle 500k+ LOC without excessive RAM
- **T1.1.4:** Cross-platform: macOS, Linux, Windows binaries

#### T1.2: Parsing & Analysis
- **T1.2.1:** Tree-sitter for AST parsing (tree-sitter-typescript, tree-sitter-javascript)
- **T1.2.2:** Custom semantic analysis on top of AST
- **T1.2.3:** Symbol resolution and reference tracking

#### T1.3: Embeddings
- **T1.3.1:** Local embedding model (all-MiniLM-L6-v2 or similar)
- **T1.3.2:** No external API dependency for core functionality
- **T1.3.3:** Efficient batch embedding generation
- **T1.3.4:** Optional: support for external embedding APIs for higher quality

#### T1.4: Storage
- **T1.4.1:** SQLite for structured data (files, symbols, relationships, findings)
- **T1.4.2:** Lancedb for vector storage and similarity search
- **T1.4.3:** All data stored in project directory (.keel/) or user config dir
- **T1.4.4:** Index format versioned for migrations

#### T1.5: File Watching
- **T1.5.1:** Use notify crate for cross-platform file watching
- **T1.5.2:** Debounce rapid changes (100-500ms)
- **T1.5.3:** Incremental re-indexing (only changed files + dependents)

### T2: CLI & TUI

#### T2.1: TUI Framework
- **T2.1.1:** Built with ratatui (Rust TUI library) for interactive mode
- **T2.1.2:** Responsive layout (adapts to terminal size)
- **T2.1.3:** Vim-style navigation (j/k) in addition to arrow keys
- **T2.1.4:** Clipboard integration via arboard crate (cross-platform)

#### T2.2: Direct Commands
- **T2.2.1:** Built with clap for argument parsing
- **T2.2.2:** Colored terminal output with severity indicators
- **T2.2.3:** Progress indicators for long operations
- **T2.2.4:** JSON output mode for scripting/integration

#### T2.3: Configuration
- **T2.3.1:** Config file: .keelrc (TOML) or keel.config.json
- **T2.3.2:** CLI flags override config file
- **T2.3.3:** Environment variable support for CI contexts

### T3: Desktop Application (Phase 2)

#### T3.1: Tauri Shell
- **T3.1.1:** Tauri for cross-platform desktop app (decision: smaller bundle, native Rust integration)
- **T3.1.2:** React or Solid for UI components (frontend framework TBD)
- **T3.1.3:** Native Rust integration via Tauri commands (no IPC overhead)

#### T3.2: Rust Integration
- **T3.2.1:** Keel core compiled directly into Tauri app (single binary)
- **T3.2.2:** Shared Rust crate between CLI and Tauri app
- **T3.2.3:** UI calls Rust functions directly via Tauri invoke API

#### T3.3: UI Performance
- **T3.3.1:** Virtualized lists for large finding sets
- **T3.3.2:** Lazy loading of code snippets
- **T3.3.3:** Responsive during background indexing

### T4: Security & Privacy

#### T4.1: Local-First
- **T4.1.1:** Zero network calls for core functionality
- **T4.1.2:** No telemetry without explicit opt-in
- **T4.1.3:** No code uploaded anywhere, ever

#### T4.2: API Key Handling (Phase 3)
- **T4.2.1:** User-provided API keys stored in system keychain
- **T4.2.2:** Keys never logged or transmitted
- **T4.2.3:** Clear documentation on what API access enables

### T5: Distribution

#### T5.1: Binaries
- **T5.1.1:** Pre-built binaries for macOS (Intel + ARM), Linux, Windows
- **T5.1.2:** Homebrew formula for macOS
- **T5.1.3:** npm package wrapping binary (for JS ecosystem familiarity)
- **T5.1.4:** Direct download from website

#### T5.2: Licensing
- **T5.2.1:** License key validation (offline-capable)
- **T5.2.2:** Trial mode: full functionality, 14-day limit
- **T5.2.3:** Graceful degradation if license expires (read-only mode)

---

## Success Criteria

### Phase 1 (MVP) — 3 Months

#### Product Metrics
- [ ] CLI analyzes 50k LOC TypeScript repo in <30 seconds
- [ ] 6 pattern detectors implemented and tested
- [ ] <20% false positive rate on test corpus (manually validated)
- [ ] Refactor plan generation produces agent-usable output

#### User Metrics
- [ ] 50 beta users (free)
- [ ] 10 paying users ($99 tier)
- [ ] Net Promoter Score >30 from beta cohort
- [ ] Clear signal on which 2-3 patterns resonate most

#### Technical Metrics
- [ ] Cross-platform builds working (macOS, Linux, Windows)
- [ ] Index size <10% of codebase size
- [ ] Incremental re-index <2 seconds for single file change

### Phase 2 (Desktop + Watch) — 6 Months

#### Product Metrics
- [ ] Watch mode stable over 8-hour sessions
- [ ] Desktop app installable and functional on all platforms
- [ ] Live monitoring shows pattern changes within 1 second of file save

#### User Metrics
- [ ] 200 total users
- [ ] 50 paying users
- [ ] $5k MRR
- [ ] 3+ public testimonials / case studies

#### Technical Metrics
- [ ] Tauri app bundle <30MB
- [ ] Memory usage <500MB during watch mode
- [ ] No crashes over 24-hour watch session

### Phase 3 (Integrations) — 9 Months

#### Product Metrics
- [ ] Git history analysis working
- [ ] At least one direct AI agent integration (Claude API or Aider)
- [ ] 10+ pattern detectors
- [ ] Python support stable

#### User Metrics
- [ ] 500 total users
- [ ] 150 paying users
- [ ] $15k MRR
- [ ] Team tier adoption (5+ teams)

### Phase 4 (Team + Scale) — 12 Months

#### Product Metrics
- [ ] CI integration available
- [ ] Shared team configuration working
- [ ] Trend tracking over time

#### User Metrics
- [ ] $30k MRR
- [ ] 1 enterprise customer
- [ ] Clear product-market fit signal (retention >80% monthly)

---

## Pricing Strategy

| Tier | Price | Features |
|------|-------|----------|
| **Trial** | Free (14 days) | Full functionality |
| **Solo** | $99 one-time | CLI, core patterns, 1 year updates |
| **Pro** | $149/year | Desktop app, all patterns, watch mode, ongoing updates |
| **Team** | $99/seat/year | Shared config, CI integration, priority support |
| **Enterprise** | Custom | On-prem support, custom patterns, SLA |

### Rationale
- One-time option captures devs who hate subscriptions
- Annual subscription for ongoing value (updates, new patterns)
- Team pricing scales with seats, aligned with value
- No free tier after trial—filters for serious users

---

## Risks & Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Embedding-based detection too noisy | Medium | High | Extensive testing on real repos; tunable thresholds; hybrid AST+embedding approach |
| Defaults too strict or too lax | Medium | Medium | Test on diverse real-world repos; ship with `strictness` knob; iterate based on feedback |
| Market too small (devs won't pay) | Medium | High | Validate with beta users before heavy investment; pivot to team/enterprise focus if needed |
| AI tools improve, problem goes away | Low | High | Problem is fundamental to LLM architecture; even if better, cleanup still needed |
| Tauri ecosystem gaps | Low | Medium | Core functionality doesn't need rich plugin ecosystem; React/Solid frontend is mature |
| Competition from IDE vendors | Medium | Medium | Move fast; focus on multi-agent compatibility; build brand as "neutral referee" |

---

## Open Questions

1. **CLI-first or Tauri-first?**
   Recommendation: CLI first. Proves core value, faster iteration, smaller surface area. Desktop in Phase 2.

2. ~~**Tauri vs Electron?**~~
   **DECIDED:** Tauri. Smaller bundle (~30MB vs ~100MB+), native Rust integration, single binary distribution. Tradeoff: smaller ecosystem, but Rust-native benefits outweigh this for our use case.

3. **Embedding model selection?**
   Need to benchmark: all-MiniLM-L6-v2 vs code-specific models (CodeBERT, etc.) for semantic similarity accuracy on code.

4. ~~**How opinionated on defaults?**~~
   **DECIDED:** Very opinionated. Ship with "pragmatic small team" defaults. Single `strictness` knob for users who want more/less noise. Per-pattern overrides for power users. No wizard.

5. **MCP integration priority?**
   Claude Desktop + MCP could be high-leverage integration. Evaluate in Phase 3 based on Claude Desktop adoption.

---

## Appendix A: Pattern Detection Details

### A1: Interface Sprawl Detection

**Signal:** Multiple inline type definitions with high embedding similarity

**Approach:**
1. Extract all type/interface definitions
2. Generate embedding for each (based on field names + structure)
3. Cluster by similarity (threshold: 0.85)
4. Flag clusters with >2 members across different files

**Example:**
```typescript
// File A
type User = { id: string; name: string; email: string }

// File B
interface UserData { id: string; name: string; email: string; }

// File C
type UserInfo = { id: string; name: string; emailAddress: string }
```

Keel flags: "3 types representing 'User' concept across 3 files"

### A2: Redundant Logic Detection

**Signal:** Function bodies with high semantic similarity

**Approach:**
1. Extract all function bodies
2. Normalize (strip comments, standardize formatting)
3. Generate embedding
4. Find pairs with similarity >0.8
5. Exclude intentional patterns (e.g., overloads)

**Tuning:** Threshold adjustable based on goals (stricter for "aggressive dedup")

### A3: God File Detection

**Signal:** Files with disproportionate exports or complexity

**Approach:**
1. Count exports per file
2. Calculate complexity metrics (lines, cyclomatic complexity proxy)
3. Flag files >2 standard deviations above mean
4. Context-aware: higher threshold for files explicitly named "utils" or "helpers"

---

## Appendix B: Refactor Plan Format

```markdown
# Keel Refactor Plan

**Generated:** 2024-12-30T10:30:00Z
**Finding ID:** sprawl-user-types-001
**Keel Version:** 1.0.0

## Issue

**Type:** Interface Sprawl
**Severity:** High
**Confidence:** 92%

Multiple inline type definitions represent the same "User" domain concept.

## Why This Matters

Given your goals (team: 4, philosophy: YAGNI-lean, lifespan: prototype):
- Duplicate types increase maintenance burden
- Changes require updates in multiple places
- Inconsistencies will emerge over time

## Locations

| File | Lines | Type Name | Similarity |
|------|-------|-----------|------------|
| src/api/types.ts | 14-18 | `User` | — |
| src/components/UserCard.tsx | 8-12 | `UserProps` | 94% |
| src/hooks/useUser.ts | 22-28 | `UserData` | 91% |
| src/api/endpoints.ts | 45-50 | `UserResponse` | 88% |

## Suggested Approach

1. Create canonical `User` interface in `src/types/user.ts`
2. Audit all fields across variants—include union of all used fields
3. Replace inline types with import in each file
4. For component props, extend or pick from User as needed
5. Remove now-unused type definitions

## Acceptance Criteria

- [ ] Single `User` type definition in `src/types/`
- [ ] No inline user-shaped types in components or hooks
- [ ] All existing functionality preserved (no type errors)
- [ ] Related imports updated

## Relevant Code

<details>
<summary>src/api/types.ts:14-18</summary>

```typescript
type User = {
  id: string;
  name: string;
  email: string;
}
```

</details>

<details>
<summary>src/components/UserCard.tsx:8-12</summary>

```typescript
interface UserProps {
  id: string;
  name: string;
  email: string;
  avatarUrl?: string;
}
```

</details>

[Additional snippets...]

## Verification

After refactoring, run:
```bash
keel scan --finding sprawl-user-types-001
```

Expected result: Finding resolved or severity reduced.

---
*Generated by Keel v1.0.0 — https://keel.dev*
```

---

## Appendix C: Competitive Landscape

| Tool | Focus | Gap Keel Fills |
|------|-------|----------------|
| ESLint | Syntax, style | No semantic patterns |
| SonarQube | Bugs, security, complexity | No goal calibration, not AI-slop focused |
| CodeClimate | Maintainability metrics | Surface-level, no refactor guidance |
| CloneDR | Syntactic duplication | Misses semantic similarity |
| Sourcegraph | Code search | Search, not analysis |
| CodeScene | Behavioral analysis | Team/org focused, not refactor workflow |
| Cursor/Aider | AI coding | Keel complements these, doesn't compete |

Keel's unique position: **Goal-calibrated semantic anti-pattern detection with AI-agent refactor integration.**

---

## Document History

| Version | Date | Author | Changes |
|---------|------|--------|---------|
| 1.0 | 2024-12-30 | — | Initial draft |
| 1.1 | 2024-12-30 | — | Decision: Tauri over Electron for desktop app |
| 1.2 | 2024-12-30 | — | Simplified goal calibration: opinionated defaults, optional strictness knob, no wizard |
| 1.3 | 2024-12-30 | — | Redesigned CLI as workflow-first TUI with keyboard shortcuts; direct commands for scripting |
