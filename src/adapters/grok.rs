//! Grok local adapter — reads ~/.grok/sessions/**/signals.json (+ summary.json).
//! Never reads auth.json or raw prompts into aggregates.
use crate::models::{AdapterStatus, SessionRecord};
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub fn grok_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".grok"));
    }
    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        let p = PathBuf::from(userprofile).join(".grok");
        if !roots.iter().any(|r| r == &p) {
            roots.push(p);
        }
    }
    roots
}

pub fn scan_grok() -> Result<(Vec<SessionRecord>, AdapterStatus)> {
    let mut sessions = Vec::new();
    let mut install_found = false;

    for root in grok_roots() {
        if !root.exists() {
            continue;
        }
        install_found = true;
        let sessions_dir = root.join("sessions");
        if !sessions_dir.is_dir() {
            continue;
        }
        for entry in WalkDir::new(&sessions_dir).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.file_name().and_then(|s| s.to_str()) != Some("signals.json") {
                continue;
            }
            match parse_session_dir(path) {
                Ok(Some(rec)) => sessions.push(rec),
                Ok(None) => {}
                Err(e) => eprintln!("argus-local: grok skip {}: {e}", path.display()),
            }
        }
    }

    let n = sessions.len();
    let status = if n > 0 {
        AdapterStatus {
            name: "Grok".into(),
            ok: true,
            partial: true, // no billable I/O / $ fields in signals
            detail: format!("{n} sessions from ~/.grok (context tokens; cost incomplete)"),
        }
    } else if install_found {
        AdapterStatus {
            name: "Grok".into(),
            ok: false,
            partial: true,
            detail: "~/.grok present but no session signals.json parsed".into(),
        }
    } else {
        AdapterStatus {
            name: "Grok".into(),
            ok: false,
            partial: true,
            detail: "no ~/.grok install found".into(),
        }
    };

    Ok((sessions, status))
}

fn parse_session_dir(signals_path: &Path) -> Result<Option<SessionRecord>> {
    let raw = fs::read_to_string(signals_path)?;
    let signals: Value = serde_json::from_str(&raw)?;

    let context_tokens = signals
        .get("contextTokensUsed")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let tool_calls = signals
        .get("toolCallCount")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let duration_secs = signals
        .get("sessionDurationSeconds")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let mut model = signals
        .get("primaryModelId")
        .and_then(|v| v.as_str())
        .or_else(|| {
            signals
                .get("modelsUsed")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
        })
        .unwrap_or("Grok")
        .to_string();

    let session_dir = signals_path.parent().unwrap_or(signals_path);
    let session_id = session_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let summary_path = session_dir.join("summary.json");
    let (started, ended, model_from_summary) = read_summary_times(&summary_path)?;
    if !model_from_summary.is_empty() {
        model = model_from_summary;
    }

    let (started_at, ended_at) = match (started, ended) {
        (Some(s), Some(e)) => (s, e),
        (Some(s), None) => {
            let e = if duration_secs > 0 {
                s + Duration::seconds(duration_secs)
            } else {
                s
            };
            (s, e)
        }
        (None, Some(e)) => {
            let s = if duration_secs > 0 {
                e - Duration::seconds(duration_secs)
            } else {
                e
            };
            (s, e)
        }
        (None, None) => {
            let ended = fs::metadata(signals_path)
                .and_then(|m| m.modified())
                .ok()
                .map(DateTime::<Utc>::from)
                .unwrap_or_else(Utc::now);
            let started = if duration_secs > 0 {
                ended - Duration::seconds(duration_secs)
            } else {
                ended
            };
            (started, ended)
        }
    };

    // Require at least some signal of a real session
    if context_tokens <= 0 && tool_calls <= 0 && duration_secs <= 0 {
        return Ok(None);
    }

    Ok(Some(SessionRecord {
        id: format!("grok:{session_id}"),
        tool: "Grok".into(),
        model,
        started_at,
        ended_at,
        // Context window usage only — not billable input/output split
        input_tokens: context_tokens,
        output_tokens: 0,
        tokens_known: context_tokens > 0,
        tools_proposed: tool_calls,
        tools_accepted: tool_calls,
        source: "grok".into(),
        cost_complete: false,
    }))
}

fn read_summary_times(
    path: &Path,
) -> Result<(Option<DateTime<Utc>>, Option<DateTime<Utc>>, String)> {
    if !path.exists() {
        return Ok((None, None, String::new()));
    }
    let raw = fs::read_to_string(path)?;
    let v: Value = serde_json::from_str(&raw)?;
    let started = v
        .get("created_at")
        .and_then(|x| x.as_str())
        .and_then(parse_rfc3339);
    let ended = v
        .get("last_active_at")
        .or_else(|| v.get("updated_at"))
        .and_then(|x| x.as_str())
        .and_then(parse_rfc3339);
    let model = v
        .get("current_model_id")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    Ok((started, ended, model))
}

fn parse_rfc3339(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}