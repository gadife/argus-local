//! argus-local - one-process local HTML MVP (Claude Code + Cursor + Grok).
mod adapters;
mod db;
mod fixtures;
mod insights;
mod models;
mod rollups;
mod server;

use adapters::scan_all;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use db::Index;
use models::{AdapterStatus, SessionRecord};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Parser, Debug)]
#[command(name = "argus-local", about = "Local-first AI session aggregates (estimates ≠ invoice)")]
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
}

fn data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("argus-local")
}

fn load_store(force_fixtures: bool) -> Result<SessionStore> {
    let mut used_fixtures = false;
    let (mut sessions, statuses) = if force_fixtures {
        used_fixtures = true;
        (fixtures::fixture_sessions(), fixtures::fixture_statuses())
    } else {
        let scan = scan_all()?;
        let mut sessions = scan.sessions;
        let mut statuses = scan.statuses;
        // Real local adapters only (claude / cursor / grok). No stub Codex etc.
        let real = sessions
            .iter()
            .filter(|s| s.source == "claude" || s.source == "cursor" || s.source == "grok")
            .count();
        if real == 0 {
            eprintln!("argus-local: no local adapter sessions found - loading fixture demo data");
            used_fixtures = true;
            sessions = fixtures::fixture_sessions();
            statuses = fixtures::fixture_statuses();
        }
        // Only tools with discovered sessions appear in statuses (see scan_all).
        (sessions, statuses)
    };

    // Persist to SQLite index
    let db_path = data_dir().join("index.sqlite");
    let index = Index::open(&db_path).context("sqlite index")?;
    index.clear()?;
    index.upsert_many(&sessions)?;
    eprintln!(
        "argus-local: indexed {} sessions at {} (fixtures={used_fixtures})",
        index.count()?,
        db_path.display()
    );

    // Reload from DB for consistency
    let since = (chrono::Utc::now() - chrono::Duration::days(90)).to_rfc3339();
    sessions = index.sessions_since(&since)?;

    Ok(SessionStore {
        sessions,
        statuses,
        used_fixtures,
    })
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
                let today = rollups::rollup(&store.sessions, days, store.statuses.clone(), store.used_fixtures);
                let insights = insights::build_insights(&store.sessions, days, store.statuses.clone(), store.used_fixtures);
                let out = serde_json::json!({
                    "today": today,
                    "insights": insights,
                    "used_fixtures": store.used_fixtures,
                    "language_lock": {
                        "estimates": "estimates ≠ invoice",
                        "observations": "observations, not a score",
                        "footer": "aggregates only · no raw prompts or code · local-first"
                    }
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(());
            }
            let url = format!("http://{bind}/");
            let shared = Arc::new(Mutex::new(store));
            if !no_open {
                // open after a brief moment so bind wins the race on slow machines
                let u = url.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(400));
                    server::open_browser(&u);
                });
            }
            server::serve(&bind, shared)?;
        }
        Commands::Scan { json } => {
            let store = load_store(false)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "sessions": store.sessions.len(),
                        "used_fixtures": store.used_fixtures,
                        "adapters": store.statuses,
                    }))?
                );
            } else {
                println!("sessions: {}", store.sessions.len());
                println!("fixtures: {}", store.used_fixtures);
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

