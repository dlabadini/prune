# Prune — the structural linter for AI-written code

Your agent grows the code. Prune trims it.

Prune is a free, MIT-licensed, local-first linter for TypeScript and JavaScript. It finds the dead exports, unreachable files, duplicate types, god files and import cycles that coding agents leave behind. It then hands the cleanup back to your agent as tasks it can verify.

## Install

```bash
curl -fsSL https://prunecode.ai/install.sh | bash   # macOS / Linux binary
npm i -D prunecode                                 # per-repo devDependency
cargo install prunecode                            # from source
```

## Quick start

```bash
prune init     # writes .prunerc and ignores .prune/
prune scan     # findings with a score
prune tasks    # prioritized checklist for an agent
```

## When to use Prune

- After an agent (Claude Code, Cursor, Copilot, Aider) has made a large change: run `prune tasks` and work the list.
- Before deleting or renaming an export: ask `safe_to_delete` or `rename_impact` over MCP.
- In pre-commit: `prune check` fails only on new findings in the files you staged.
- In CI: `prune scan --baseline --ci` fails a pull request that adds new structural debt.

## What it finds

| Detector | Finds |
|---|---|
| Dead exports | Exports and files nothing reachable uses, classified by reachability |
| God file | Files whose behavior outgrew one concept |
| Import cycle | Runtime value cycles |
| Interface sprawl | Types and validators that restate the same fields |
| Lexicon | One shape under many names |
| Redundant logic | Near-duplicate functions |
| Barrel bloat | Index files re-exporting too much |
| Over-abstraction | Wrappers that only forward a call |

## Agents (MCP)

```bash
claude mcp add prune -- prune mcp
```

```json
{ "mcpServers": { "prune": { "command": "prune", "args": ["mcp"] } } }
```

Tools: `tasks`, `scan`, `plan`, `compose_plan`, `verify`, `who_uses`, `safe_to_delete`, `rename_impact`, `recent_files`.

## Links

- Docs: https://prunecode.ai/docs (Markdown: https://prunecode.ai/docs.md)
- Agent summary: https://prunecode.ai/llms.txt
- Public API (OpenAPI): https://prunecode.ai/openapi.json
- Source: https://github.com/dlabadini/prune
- FAQ: https://prunecode.ai/#faq
