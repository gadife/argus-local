# Argus Local

One-process **local-first** dashboard for engineers. A single Rust binary (`argus-local`) reads local Claude Code / Cursor / Grok session data from disk (absent tools like Codex are hidden — no stub pills), indexes into SQLite, rolls up metrics + deterministic insights, and serves an embedded HTML UI on `127.0.0.1` — or a full-screen terminal UI via `argus-local tui`.

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

# Full-screen terminal UI (Today / Insights + nav stubs) — same engine as HTML
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

If no Claude/Cursor/Grok sessions are found, **fixture demo data** loads automatically so the UI always demos.

## UI

- **Today** — KPI strip, By tool, Activity, Shipping opt-in stub
- **Insights** — 3-5 deterministic findings with evidence + Context panel
- Stub nav: Activity / Tools / Shipping / Share / Settings

### Terminal UI (`argus-local tui`)

Same data path as HTML (adapters → SQLite → rollups/insights). Renders whole screens in the terminal with keyboard nav — not a JSON dump. Built with **ratatui + crossterm**.

## Out of scope (v1)

Real Codex adapter, org Share bridge, GitHub shipping join, native GUI, Node sidecar, Docker.

## License

MIT

## Proof screenshots

See `proof/today.png` and `proof/insights.png` from `argus-local open --fixtures` on gili-pc.

TUI proofs: `proof/tui-today.txt` / `proof/tui-today.png` and `proof/tui-insights.txt` / `proof/tui-insights.png` from `argus-local tui --fixtures --proof proof/`.
