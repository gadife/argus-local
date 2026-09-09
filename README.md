# Argus Local

One-process **local-first** dashboard for engineers. A single Rust binary (`argus-local`) reads local Claude Code / Cursor / Grok session data from disk (absent tools like Codex are hidden — no stub pills), optionally joins **opt-in GitHub shipping** (merged PRs / commits / files), indexes into SQLite, rolls up metrics + deterministic insights, and serves an embedded HTML UI on `127.0.0.1` — or a full-screen terminal UI via `argus-local tui`.

> **Language lock**
> - `estimates ≠ invoice`
> - `observations, not a score`
> - No Elite/Low labels, peer ranks, productivity scores, or causal ROI
> - Shipping (when shown): `correlation with sessions — not a productivity score.`
> - Footer: `aggregates only — no raw prompts or code — local-first`
> - Share to org defaults **Off** (UI only in v1)

## Requirements

- Rust stable (1.70+)
- Windows / macOS / Linux
- Optional: authenticated [`gh`](https://cli.github.com/) CLI for GitHub shipping

## Build

```bash
cargo build --release
```

Binary: `target/release/argus-local` (`.exe` on Windows).

## Run

```bash
# Scan adapters, serve UI, open browser
cargo run --release -- open

# Or after install / from target
./target/release/argus-local open

# JSON only (no server)
argus-local open --json

# Force fixture demo data
argus-local open --fixtures

# Full-screen terminal UI (Today / Insights / Shipping / Settings) — same engine as HTML
argus-local tui
argus-local tui --fixtures
argus-local tui --days 7
# Keys: Tab/h/l navigate · 1–7 pages · d cycle period · q quit

# Non-interactive TUI screen dumps (proof)
argus-local tui --fixtures --proof proof/

# Scan status
argus-local scan
argus-local scan --json
```

Default bind: `http://127.0.0.1:8787/`

SQLite index: `%LOCALAPPDATA%/argus-local/index.sqlite` (Windows) or platform equivalent via `dirs::data_local_dir`.

## Adapters (v1)

| Tool | Source | Notes |
|------|--------|-------|
| Claude Code | `~/.claude/**/*.jsonl` (+ best-effort AppData) | Real parser |
| Cursor | `state.vscdb` / ai-tracking under Cursor User storage | Best-effort; copies DB to avoid locks |
| Codex | Hidden when absent | No local adapter in v1 — never invent stub rows |
| Grok | `~/.grok/sessions/**/signals.json` (+ summary) | Live from disk; context tokens; marked **cost incomplete** |
| GitHub shipping | Opt-in (`gh` and/or PAT) | Merged PRs / commits / files for selected period; **hidden counts when unconfigured** |

If no Claude/Cursor/Grok sessions are found, **fixture demo data** loads automatically so the UI always demos. GitHub shipping stays absent unless opted in.

## Configure GitHub shipping (opt-in)

Shipping never invents counts. When unconfigured, the Shipping panel shows dashes / an **unconfigured** state.

Pick **one** (secrets stay on this machine — **never commit PATs** or tokens to the repo):

1. **Env PAT:** set `ARGUS_GITHUB_TOKEN` to a personal access token (`repo` scope is enough for private PR search). Presence of the token opts in.
2. **Env flag + gh:** set `ARGUS_GITHUB_ENABLED=1` (or `true`) and authenticate with `gh auth login`. Argus prefers `gh api` when `gh` is available.
3. **Local settings file** (no secrets required in the file): write `%LOCALAPPDATA%/argus-local/settings.json` (or `$XDG_DATA_HOME/argus-local/settings.json`):

```json
{ "github": { "enabled": true } }
```

Then use authenticated `gh`, or set `ARGUS_GITHUB_TOKEN`. The settings path is documented in the Settings page of the UI / TUI.

## UI

- **Today** — KPI strip, By tool, Activity, Shipping (real or unconfigured)
- **Insights** — 3–5 deterministic findings with evidence + Context panel
- **Shipping** — merged PRs, commits, files touched for the selected period
- **Settings** — how to opt in to GitHub; Share Off reminder
- Stub nav: Activity / Tools / Share

### Terminal UI (`argus-local tui`)

Same data path as HTML (adapters → SQLite → rollups/insights). Renders whole screens in the terminal with keyboard nav — not a JSON dump. Built with **ratatui + crossterm**.

## Out of scope (this ticket / v1)

Linear adapter, Jira, org Share bridge, inventing shipping when GitHub isn’t configured, real Codex adapter, native GUI, Node sidecar, Docker.

## License

MIT

## Proof screenshots

See `proof/` for HTML + TUI Shipping screens (configured + unconfigured) attached on Linear ARG-32.
