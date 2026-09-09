//! Adapter discovery and ingestion.
mod claude;
mod cursor;
mod grok;

pub use claude::scan_claude;
pub use cursor::scan_cursor;
pub use grok::scan_grok;

use crate::models::{AdapterStatus, SessionRecord};
use anyhow::Result;

pub struct ScanResult {
    pub sessions: Vec<SessionRecord>,
    pub statuses: Vec<AdapterStatus>,
    pub used_fixtures: bool,
}

/// Only include adapter status badges for tools that produced real session rows.
/// Absent tools (e.g. Codex with no local adapter/data) are omitted entirely.
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

    Ok(ScanResult {
        sessions,
        statuses,
        used_fixtures: false,
    })
}