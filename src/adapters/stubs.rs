//! Codex / Grok stubs for v1.
use crate::models::{AdapterStatus, SessionRecord};

pub fn stub_codex() -> (Vec<SessionRecord>, AdapterStatus) {
    (
        vec![],
        AdapterStatus {
            name: "Codex".into(),
            ok: true,
            partial: false,
            detail: "stub adapter (no local scan in v1)".into(),
        },
    )
}

pub fn stub_grok() -> (Vec<SessionRecord>, AdapterStatus) {
    (
        vec![],
        AdapterStatus {
            name: "Grok".into(),
            ok: false,
            partial: true,
            detail: "stub · cost incomplete (no billable I/O in v1)".into(),
        },
    )
}
