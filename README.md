# Argus Local

One-process **local-first** dashboard for engineers. A single Rust binary (`argus-local`) reads local Claude Code / Cursor session data (Codex/Grok stubs in v1), indexes into SQLite, rolls up metrics + deterministic insights, and serves an embedded HTML UI on `127.0.0.1`.

> **Language lock**
> - `estimates ≠ invoice`
> - `observations, not a score`
> - No Elite/Low labels, peer ranks, productivity scores, or causal ROI
> - Shipping (when shown): `correlation with sessions · not a productivity score.`
> - Footer: `aggregates only · no raw prompts or code · local-first`
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
| Codex | Stub | Empty unless fixtures |
| Grok | Stub | Marked **cost incomplete** |

If no Claude/Cursor sessions are found, **fixture demo data** loads automatically so the UI always demos.

## UI

- **Today** — KPI strip, By tool, Activity, Shipping opt-in stub
- **Insights** — 3–5 deterministic findings with evidence + Context panel
- Stub nav: Activity / Tools / Shipping / Share / Settings

## Out of scope (v1)

Real Codex/Grok adapters, org Share bridge, GitHub shipping join, native GUI, Node sidecar, Docker.

## License

MIT
