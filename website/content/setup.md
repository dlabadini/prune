# Set up Prune in this repository (instructions for coding agents)

You are setting up Prune, a local structural linter for TypeScript and JavaScript, in the repository you are working in. Follow these steps in order. Each step says how to check it worked. Do not ask the user about choices this page already makes; report what you did at the end.

## 1. Install Prune

Check whether it is already installed: `prune --version`, or `npx --no-install prune --version` in a Node project. Always pass `--no-install` before Prune is installed, because a bare `npx prune` would download an unrelated npm package named `prune`.

If it isn't installed:

- **Node project** (a `package.json` exists): install it as a devDependency with the repo's package manager. Use the lockfile to tell which one:
  - `bun.lock` / `bun.lockb` → `bun add -d prunecode`
  - `pnpm-lock.yaml` → `pnpm add -D prunecode`
  - `yarn.lock` → `yarn add -D prunecode`
  - otherwise → `npm i -D prunecode`

  From then on, run Prune as `npx prune` (or `bunx prune`, `pnpm exec prune`, `yarn prune`).
- **No package.json**: `curl -fsSL https://prunecode.ai/install.sh | bash`, then use `~/.prune/bin/prune`, or add that directory to PATH.

Check: `npx prune --version` (or `prune --version`) prints a version.

## 2. Initialize and scan

```bash
npx prune init    # writes .prunerc, adds .prune/ to .gitignore
npx prune scan    # first scan; builds the local index in .prune/
```

If the scan prints "Suggested .prunerc excludes", add the ones that are clearly generated or vendored code (build output, generated clients, fixtures) to `exclude` in `.prunerc`, then scan again.

Check: `.prunerc` exists, `.prune/` is in `.gitignore`, and `prune scan` prints a score.

## 3. Record a baseline

Existing findings shouldn't block anyone's commits. Record them:

```bash
npx prune baseline   # writes .prune-baseline.json
```

Check: `.prune-baseline.json` exists. It should be committed.

## 4. Add the pre-commit check

Use the hook system the repo already has. Never add a second one.

- **husky** (`.husky/` exists): append `npx prune check` as its own line to `.husky/pre-commit`.
- **lefthook** (`lefthook.yml` exists): add under `pre-commit.commands`:
  ```yaml
  prune:
    glob: "*.{ts,tsx,js,jsx}"
    run: npx prune check {staged_files}
  ```
- **pre-commit** (`.pre-commit-config.yaml` exists): add a local hook, which works whether Prune came from npm or the binary install:
  ```yaml
  - repo: local
    hooks:
      - id: prune
        name: prune check
        entry: npx prune check
        language: system
        types_or: [ts, tsx, javascript, jsx]
        require_serial: true
  ```
  (Without npm, use `entry: prune check`.)
- **lint-staged** only: add `"*.{ts,tsx,js,jsx}": "prune check"` to its config.
- **None of these**: leave hooks alone and say so in your report. The user can opt in later.

`prune check` fails only on new findings at or above `high` severity, and it never downloads anything, so it is safe in a hook.

Check: stage a TypeScript file and run `npx prune check`. It prints "passed" or lists new findings.

## 5. Connect the MCP server

Register `prune mcp` with the agent client you are running in. Use `npx prune` as the command if Prune was installed with npm:

- **Claude Code**: `claude mcp add prune -- npx prune mcp`
- **Cursor**: add to `.cursor/mcp.json`
- **Other clients that read `.mcp.json`**: add to `.mcp.json`

```json
{ "mcpServers": { "prune": { "command": "npx", "args": ["prune", "mcp"] } } }
```

Check: the client lists tools such as `tasks`, `verify`, `safe_to_delete` and `rename_impact`.

## 6. Leave instructions for future agents

Append to `AGENTS.md` (or `CLAUDE.md` if that is what the repo uses; create `AGENTS.md` if neither exists):

```markdown
## Prune
- Before deleting or renaming an export, ask Prune: `safe_to_delete` / `rename_impact` (MCP) or `npx prune rename … ` as a dry run.
- After a refactor, run `npx prune check --fail-on medium` and fix what it reports.
- To find cleanup work, run `npx prune tasks`. Each task ends with a `prune verify <id>` command that must exit 0.
```

## 7. Work the first tasks (only if the user asked for cleanup)

```bash
npx prune tasks -n 10
```

For each task, in order:
1. If the task has an `autofix` command, run it with the repo's typecheck: `npx prune fix --apply --check "<typecheck command>" --path-prefix <file>`.
2. Otherwise make the change described in `change`. Before deleting anything, confirm with `safe_to_delete`.
3. Run the task's `done when` command (`npx prune verify <id>`) and the repo's typecheck and tests. Move on only when both pass.

## 8. Report

Tell the user, briefly:
- how Prune was installed
- which hook and MCP client you configured, or why you skipped one
- how many findings the baseline recorded
- the top three tasks from `prune tasks`

Then stop. Don't commit unless the user asked you to.

Reference: [Docs](https://prunecode.ai/docs) · [llms.txt](https://prunecode.ai/llms.txt) · [Source](https://github.com/dlabadini/prune)
