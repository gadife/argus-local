//! argus-local - one-process local HTML + TUI MVP (Claude Code + Cursor + Grok + opt-in GitHub shipping).
mod adapters;
mod db;
mod fixtures;
mod insights;
mod models;
mod rollups;
mod server;
mod settings;
mod tui;

use adapters::scan_all;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use db::Index;
use models::{AdapterStatus, SessionRecord, ShippingEvent};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Parser, Debug)]
#[command(name = "argus-local", about = "Local-first AI session aggregates (estimates != invoice)")]
struct Cli {
    #[command(subcommand)]
    cmd: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Scan local adapters, serve UI on 127.0.0.1, open browser
    Open {
        /// Emit dashboard JSON to stdout instead of serving HTML
        #[arg(long)]
        json: bool,
        /// Bind address (default 127.0.0.1:8787)
        #[arg(long, default_value = "127.0.0.1:8787")]
        bind: String,
        /// Force fixture demo data even if adapters find sessions
        #[arg(long)]
        fixtures: bool,
        /// Period days for --json
        #[arg(long, default_value_t = 7)]
        days: u32,
        /// Do not open a browser tab
        #[arg(long)]
        no_open: bool,
    },
    /// Full-screen terminal UI (Today / Insights / Shipping) — same engine as HTML
    Tui {
        /// Force fixture demo data even if adapters find sessions
        #[arg(long)]
        fixtures: bool,
        /// Period days (1 / 7 / 30; press d in TUI to cycle)
        #[arg(long, default_value_t = 7)]
        days: u32,
        /// Write Today + Insights + Shipping screen dumps to DIR (non-interactive proof) then exit
        #[arg(long, value_name = "DIR")]
        proof: Option<PathBuf>,
    },
    /// Scan only and print adapter status
    Scan {
        #[arg(long)]
        json: bool,
    },
}

pub struct SessionStore {
    pub sessions: Vec<SessionRecord>,
    pub statuses: Vec<AdapterStatus>,
    pub used_fixtures: bool,
    pub shipping_events: Vec<ShippingEvent>,
    pub github_configured: bool,
    pub github_auth_source: Option<String>,
    pub github_login: Option<String>,
}

fn data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("argus-local")
}

fn load_store(force_fixtures: bool) -> Result<SessionStore> {
    let mut used_fixtures = false;
    let mut shipping_events = Vec::new();
    let mut github_configured = false;
    let mut github_auth_source = None;
    let mut github_login = None;

    let (mut sessions, mut statuses) = if force_fixtures {
        used_fixtures = true;
        (fixtures::fixture_sessions(), fixtures::fixture_statuses())
    } else {
        let scan = scan_all()?;
        let mut sessions = scan.sessions;
        let mut statuses = scan.statuses;
        shipping_events = scan.shipping_events;
        github_configured = scan.github_configured;
        github_auth_source = scan.github_auth_source;
        github_login = scan.github_login;
        // Real local adapters only (claude / cursor / grok). No stub Codex etc.
        let real = sessions
            .iter()
            .filter(|s| s.source == "claude" || s.source == "cursor" || s.source == "grok")
            .count();
        if real == 0 {
            eprintln!("argus-local: no local adapter sessions found - loading fixture demo data");
            used_fixtures = true;
            sessions = fixtures::fixture_sessions();
            // Keep GitHub status if present; replace session tool statuses with fixtures.
            let gh = statuses
                .iter()
                .find(|s| s.name == "GitHub")
                .cloned();
            statuses = fixtures::fixture_statuses();
            if let Some(g) = gh {
                statuses.push(g);
            }
        }
        (sessions, statuses)
    };

    // Persist to SQLite index
    let db_path = data_dir().join("index.sqlite");
    let index = Index::open(&db_path).context("sqlite index")?;
    index.clear()?;
    index.upsert_many(&sessions)?;
    index.upsert_shipping(&shipping_events)?;
    eprintln!(
        "argus-local: indexed {} sessions + {} shipping events at {} (fixtures={used_fixtures}, github_configured={github_configured})",
        index.count()?,
        index.shipping_count()?,
        db_path.display()
    );

    // Reload from DB for consistency
    let since = (chrono::Utc::now() - chrono::Duration::days(90)).to_rfc3339();
    sessions = index.sessions_since(&since)?;
    shipping_events = index.shipping_events()?;

    Ok(SessionStore {
        sessions,
        statuses,
        used_fixtures,
        shipping_events,
        github_configured,
        github_auth_source,
        github_login,
    })
}

fn shipping_ctx(store: &SessionStore) -> rollups::ShippingContext<'_> {
    rollups::ShippingContext {
        configured: store.github_configured,
        events: &store.shipping_events,
        auth_source: store.github_auth_source.as_deref(),
        login: store.github_login.as_deref(),
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Commands::Open {
            json,
            bind,
            fixtures,
            days,
            no_open,
        } => {
            let store = load_store(fixtures)?;
            if json {
                let ship = shipping_ctx(&store);
                let today = rollups::rollup(
                    &store.sessions,
                    days,
                    store.statuses.clone(),
                    store.used_fixtures,
                    ship,
                );
                let insights = insights::build_insights(
                    &store.sessions,
                    days,
                    store.statuses.clone(),
                    store.used_fixtures,
                    rollups::ShippingContext {
                        configured: store.github_configured,
                        events: &store.shipping_events,
                        auth_source: store.github_auth_source.as_deref(),
                        login: store.github_login.as_deref(),
                    },
                );
                let out = serde_json::json!({
                    "today": today,
                    "insights": insights,
                    "used_fixtures": store.used_fixtures,
                    "github_configured": store.github_configured,
                    "prompts_enabled": settings::prompts_enabled(),
                    "language_lock": {
                        "estimates": "estimates != invoice",
                        "observations": "observations, not a score",
                        "shipping": "correlation with sessions — not a productivity score",
                        "footer": "aggregates only — no raw prompts or code — local-first"
                    }
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(());
            }
            let url = format!("http://{bind}/");
            let shared = Arc::new(Mutex::new(store));
            if !no_open {
                let u = url.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(400));
                    server::open_browser(&u);
                });
            }
            server::serve(&bind, shared)?;
        }
        Commands::Tui {
            fixtures,
            days,
            proof,
        } => {
            let store = load_store(fixtures)?;
            if let Some(dir) = proof {
                let paths = tui::write_proof_screens(store, days, &dir)?;
                for p in paths {
                    eprintln!("argus-local: wrote {}", p.display());
                }
                return Ok(());
            }
            tui::run(store, days)?;
        }
        Commands::Scan { json } => {
            let store = load_store(false)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "sessions": store.sessions.len(),
                        "used_fixtures": store.used_fixtures,
                        "github_configured": store.github_configured,
                        "shipping_events": store.shipping_events.len(),
                        "adapters": store.statuses,
                    }))?
                );
            } else {
                println!("sessions: {}", store.sessions.len());
                println!("fixtures: {}", store.used_fixtures);
                println!("github_configured: {}", store.github_configured);
                println!("shipping_events: {}", store.shipping_events.len());
                for s in store.statuses {
                    println!(
                        "- {} ok={} partial={} ({})",
                        s.name, s.ok, s.partial, s.detail
                    );
                }
            }
        }
    }
    Ok(())
}
