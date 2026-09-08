//! Adapter discovery and ingestion.
mod claude;
mod cursor;
mod stubs;

pub use claude::scan_claude;
pub use cursor::scan_cursor;
pub use stubs::{stub_codex, stub_grok};

use crate::models::{AdapterStatus, SessionRecord};
use anyhow::Result;

pub struct ScanResult {
    pub sessions: Vec<SessionRecord>,
    pub statuses: Vec<AdapterStatus>,
    pub used_fixtures: bool,
}

pub fn scan_all() -> Result<ScanResult> {
    let mut sessions = Vec::new();
    let mut statuses = Vec::new();

    match scan_claude() {
        Ok((rows, status)) => {
            sessions.extend(rows);
            statuses.push(status);
        }
        Err(e) => statuses.push(AdapterStatus {
            name: "Claude Code".into(),
            ok: false,
            partial: true,
            detail: format!("scan error: {e}"),
        }),
    }

    match scan_cursor() {
        Ok((rows, status)) => {
            sessions.extend(rows);
            statuses.push(status);
        }
        Err(e) => statuses.push(AdapterStatus {
            name: "Cursor".into(),
            ok: false,
            partial: true,
            detail: format!("scan error: {e}"),
        }),
    }

    // Stubs for Codex / Grok in v1
    let (codex_rows, codex_status) = stub_codex();
    sessions.extend(codex_rows);
    statuses.push(codex_status);

    let (grok_rows, grok_status) = stub_grok();
    sessions.extend(grok_rows);
    statuses.push(grok_status);

    let used_fixtures = false;
    Ok(ScanResult {
        sessions,
        statuses,
        used_fixtures,
    })
}
