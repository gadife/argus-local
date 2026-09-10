//! Adapter discovery and ingestion.
pub(crate) mod claude;
mod prompts;
pub(crate) mod cursor;
mod github;
mod grok;
mod coaching;
mod telemetry;

pub use claude::scan_claude;
pub use cursor::scan_cursor;
pub use github::{is_opted_in, scan_github, settings_path_display, snapshot_for_period};
pub use grok::scan_grok;
pub use prompts::{load_session_texts, SessionTexts, FIXTURE_PROMPTS_ID};
pub use coaching::{coaching_has_any, load_session_coaching};
pub use telemetry::load_session_telemetry;

use crate::models::{AdapterStatus, SessionRecord, ShippingEvent};
use anyhow::Result;

pub struct ScanResult {
    pub sessions: Vec<SessionRecord>,
    pub statuses: Vec<AdapterStatus>,
    pub used_fixtures: bool,
    pub shipping_events: Vec<ShippingEvent>,
    pub github_configured: bool,
    pub github_auth_source: Option<String>,
    pub github_login: Option<String>,
}

/// Only include adapter status badges for tools that produced real session rows.
/// Absent tools (e.g. Codex with no local adapter/data) are omitted entirely.
/// GitHub shipping is opt-in and separate from session tool pills.
pub fn scan_all() -> Result<ScanResult> {
    let mut sessions = Vec::new();
    let mut statuses = Vec::new();

    match scan_claude() {
        Ok((rows, status)) => {
            if !rows.is_empty() {
                statuses.push(status);
                sessions.extend(rows);
            }
        }
        Err(e) => eprintln!("argus-local: claude scan error: {e}"),
    }

    match scan_cursor() {
        Ok((rows, status)) => {
            if !rows.is_empty() {
                statuses.push(status);
                sessions.extend(rows);
            }
        }
        Err(e) => eprintln!("argus-local: cursor scan error: {e}"),
    }

    match scan_grok() {
        Ok((rows, status)) => {
            if !rows.is_empty() {
                statuses.push(status);
                sessions.extend(rows);
            }
        }
        Err(e) => eprintln!("argus-local: grok scan error: {e}"),
    }

    // Codex and other absent tools: no stub rows / status pills.

    let (shipping_events, github_configured, github_auth_source, github_login) =
        match scan_github() {
            Ok(g) => {
                // Surface GitHub status only when opted in (configured path).
                if g.configured {
                    statuses.push(g.status.clone());
                }
                (
                    g.events,
                    g.configured,
                    g.auth_source,
                    g.login,
                )
            }
            Err(e) => {
                eprintln!("argus-local: github scan error: {e}");
                (vec![], false, None, None)
            }
        };

    Ok(ScanResult {
        sessions,
        statuses,
        used_fixtures: false,
        shipping_events,
        github_configured,
        github_auth_source,
        github_login,
    })
}