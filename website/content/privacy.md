# Privacy

Last updated: October 2026.

Prune is built to be local-first, and this page covers both the Prune tool and this website.

## The Prune tool (CLI, TUI, pre-commit hook, MCP server)

- **Your code never leaves your machine.** Prune parses, indexes and analyzes code locally. The index is stored in the `.prune/` directory inside your project.
- **No telemetry.** Prune does not collect usage data, crash reports, or analytics of any kind, and a scan makes no network calls.
- **No account.** There is no sign-up, license key, or login.
- **Optional model download.** If you enable semantic matching, Prune downloads an open embedding model (all-MiniLM-L6-v2) once and stores it in `.prune/models`. The model runs locally; your code is not sent anywhere to compute embeddings.
- **Install scripts.** The install script and package managers (npm, crates.io, GitHub Releases) download the Prune binary from those services, subject to their own privacy policies.

## This website (prunecode.ai)

- **Analytics.** This site uses Vercel Web Analytics to count page views. It does not use cookies, and it does not track you across sites or build a profile of you. It records aggregate information such as the page visited, referrer, browser, and country.
- **Hosting logs.** The site is hosted on Vercel, which may keep standard server logs (such as IP address and user agent) for security and operations, per Vercel's privacy policy.
- **No forms, no accounts.** This site does not collect personal information through forms, newsletters or accounts.

## Contact

Questions about privacy: hello@prune.dev, or open an issue at https://github.com/dlabadini/prune/issues.
