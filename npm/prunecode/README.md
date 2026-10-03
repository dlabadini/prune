# prunecode

npm distribution of [Prune](https://prunecode.ai), the free, local-first structural linter for
AI-written TypeScript. It finds dead exports, god files, duplicate types, and import cycles, and
hands the cleanup back to your agent as verifiable tasks.

```bash
npm i -D prunecode
npx prune init
npx prune scan
```

Pre-commit with husky (`.husky/pre-commit`):

```bash
npx prune check
```

Agents (MCP): `claude mcp add prune -- npx prune mcp`

See the [README](https://github.com/dlabadini/prune#readme) for everything else. MIT licensed.
