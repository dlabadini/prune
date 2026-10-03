# About Prune

Prune is an open-source structural linter for TypeScript and JavaScript codebases, built for the way software is written now: with coding agents.

## Why it exists

Coding agents write code that compiles, passes lint and works. They also leave behind a kind of mess that line-level tools don't see. They write a second `User` type because they didn't find the first one. They leave a helper exported after its last caller moved. They create a file nothing imports anymore. Each change looks fine in review. Six months later the codebase is twice the size it needs to be, and every agent that reads it gets more confused.

Prune reads the whole module graph instead of one file at a time. It finds exports and files that nothing reachable uses, the same shape declared under different names, files that grew into several concepts, and runtime import cycles. Then it does the part that matters for agent workflows: it turns each finding into a precise task with a command that proves the task is done, so the same agent that made the mess can clean it up safely.

## How it's built

Prune is a single Rust binary. It parses with Oxc, matches structure with ast-grep, keeps a local SQLite index, and optionally uses a small local embedding model for semantic similarity. It runs as a CLI, an interactive TUI, a pre-commit hook and an MCP server. Nothing is uploaded anywhere.

## Who makes it

Prune is created and maintained by Devin Labadini, with contributions from the open-source community. It is MIT licensed and free for any use. The source, issue tracker and roadmap live on GitHub: https://github.com/dlabadini/prune.

Accuracy matters more than finding count. Every detector is covered by labeled fixtures that check both precision and recall, and false-positive reports are treated as bugs.
