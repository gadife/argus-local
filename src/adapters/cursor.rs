//! Cursor local DB adapter — best-effort state.vscdb / ai-tracking.
use crate::models::{AdapterStatus, SessionRecord};
use anyhow::Result;
use chrono::{TimeZone, Utc};
use rusqlite::Connection;
use std::path::PathBuf;
use walkdir::WalkDir;

pub fn cursor_candidate_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(appdata) = std::env::var("APPDATA") {
        let base = PathBuf::from(&appdata).join("Cursor").join("User");
        dirs.push(base.join("globalStorage"));
        dirs.push(base.join("workspaceStorage"));
    }
    if let Ok(home) = std::env::var("HOME") {
        // macOS
        dirs.push(
            PathBuf::from(&home)
                .join("Library")
                .join("Application Support")
                .join("Cursor")
                .join("User")
                .join("globalStorage"),
        );
        dirs.push(
            PathBuf::from(&home)
                .join("Library")
                .join("Application Support")
                .join("Cursor")
                .join("User")
                .join("workspaceStorage"),
        );
        // Linux
        dirs.push(
            PathBuf::from(&home)
                .join(".config")
                .join("Cursor")
                .join("User")
                .join("globalStorage"),
        );
        dirs.push(
            PathBuf::from(&home)
                .join(".config")
                .join("Cursor")
                .join("User")
                .join("workspaceStorage"),
        );
    }
    if let Some(home) = dirs::home_dir() {
        dirs.push(
            home.join(".cursor")
                .join("User")
                .join("globalStorage"),
        );
    }
    dirs
}

pub(crate) fn find_vscdb_files_for_prompts() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in cursor_candidate_dirs() {
        if !dir.exists() {
            continue;
        }
        for entry in WalkDir::new(&dir)
            .max_depth(6)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if name == "state.vscdb"
                || name.contains("ai-tracking")
                || name.ends_with(".vscdb")
            {
                out.push(p.to_path_buf());
            }
        }
    }
    out
}

pub fn scan_cursor() -> Result<(Vec<SessionRecord>, AdapterStatus)> {
    let files = find_vscdb_files_for_prompts();
    if files.is_empty() {
        return Ok((
            vec![],
            AdapterStatus {
                name: "Cursor".into(),
                ok: false,
                partial: true,
                detail: "no state.vscdb / ai-tracking DB found".into(),
            },
        ));
    }

    let mut sessions = Vec::new();
    let mut errors = 0usize;
    for db_path in &files {
        match read_cursor_db(db_path) {
            Ok(mut rows) => sessions.append(&mut rows),
            Err(e) => {
                errors += 1;
                eprintln!("argus-local: cursor skip {}: {e}", db_path.display());
            }
        }
    }

    // Dedupe by id
    sessions.sort_by(|a, b| a.id.cmp(&b.id));
    sessions.dedup_by(|a, b| a.id == b.id);

    let n = sessions.len();
    let partial = n == 0 || errors > 0;
    Ok((
        sessions,
        AdapterStatus {
            name: "Cursor".into(),
            ok: n > 0,
            partial,
            detail: format!(
                "{n} sessions from {} db files ({} errors)",
                files.len(),
                errors
            ),
        },
    ))
}

fn read_cursor_db(path: &PathBuf) -> Result<Vec<SessionRecord>> {
    // Copy to temp to avoid lock contention with running Cursor
    let tmp = std::env::temp_dir().join(format!(
        "argus-cursor-{}.vscdb",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("db")
    ));
    std::fs::copy(path, &tmp)?;
    let conn = Connection::open(&tmp)?;
    let mut sessions = Vec::new();

    // ItemTable key/value store (VS Code style)
    if table_exists(&conn, "ItemTable") {
        sessions.extend(from_item_table(&conn)?);
    }
    // Some builds use cursorDiskKV
    if table_exists(&conn, "cursorDiskKV") {
        sessions.extend(from_cursor_disk_kv(&conn)?);
    }

    let _ = std::fs::remove_file(&tmp);
    Ok(sessions)
}

fn table_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        [name],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .unwrap_or(false)
}

fn from_item_table(conn: &Connection) -> Result<Vec<SessionRecord>> {
    let mut stmt = conn.prepare("SELECT key, value FROM ItemTable")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut out = Vec::new();
    for r in rows.flatten() {
        let (key, value) = r;
        let key_l = key.to_lowercase();
        if !(key_l.contains("ai")
            || key_l.contains("composer")
            || key_l.contains("aichat")
            || key_l.contains("bubble")
            || key_l.contains("generation"))
        {
            continue;
        }
        if let Some(s) = parse_cursor_blob(&key, &value) {
            out.push(s);
        }
    }
    Ok(out)
}

fn from_cursor_disk_kv(conn: &Connection) -> Result<Vec<SessionRecord>> {
    let mut stmt = conn.prepare("SELECT key, value FROM cursorDiskKV")?;
    let rows = stmt.query_map([], |row| {
        let key: String = row.get(0)?;
        let value: Vec<u8> = row.get(1)?;
        let s = String::from_utf8_lossy(&value).to_string();
        Ok((key, s))
    })?;
    let mut out = Vec::new();
    for r in rows.flatten() {
        let (key, value) = r;
        if let Some(s) = parse_cursor_blob(&key, &value) {
            out.push(s);
        }
    }
    Ok(out)
}

fn parse_cursor_blob(key: &str, value: &str) -> Option<SessionRecord> {
    let v: serde_json::Value = serde_json::from_str(value).ok()?;
    let model = v
        .get("model")
        .or_else(|| v.pointer("/modelName"))
        .or_else(|| v.pointer("/model/name"))
        .and_then(|x| x.as_str())
        .unwrap_or("Cursor")
        .to_string();

    let input = v
        .pointer("/tokenCount/inputTokens")
        .or_else(|| v.pointer("/usage/input_tokens"))
        .or_else(|| v.get("inputTokens"))
        .and_then(|x| x.as_i64())
        .unwrap_or(0);
    let output = v
        .pointer("/tokenCount/outputTokens")
        .or_else(|| v.pointer("/usage/output_tokens"))
        .or_else(|| v.get("outputTokens"))
        .and_then(|x| x.as_i64())
        .unwrap_or(0);

    // Skip empty blobs
    if input + output == 0 && v.get("bubbles").is_none() && v.get("messages").is_none() {
        // Still accept composer tabs with timestamps as lightweight sessions
        if v.get("createdAt").is_none() && v.get("timestamp").is_none() {
            return None;
        }
    }

    let created_ms = v
        .get("createdAt")
        .or_else(|| v.get("timestamp"))
        .or_else(|| v.get("lastUpdatedAt"))
        .and_then(|x| x.as_i64())
        .unwrap_or_else(|| Utc::now().timestamp_millis());

    let started = Utc
        .timestamp_millis_opt(created_ms)
        .single()
        .unwrap_or_else(Utc::now);
    let ended = started + chrono::Duration::minutes(30);

    let proposed = v
        .get("toolsProposed")
        .and_then(|x| x.as_i64())
        .unwrap_or(0);
    let accepted = v
        .get("toolsAccepted")
        .and_then(|x| x.as_i64())
        .unwrap_or(proposed);

    let id = format!(
        "cursor:{}",
        blake_short(&format!("{key}:{created_ms}:{input}:{output}"))
    );

    // Never invent token counts — unknown stays unknown (UI shows —).
    let tokens_known = input + output > 0;
    Some(SessionRecord {
        id,
        tool: "Cursor".into(),
        model,
        started_at: started,
        ended_at: ended,
        input_tokens: if tokens_known { input } else { 0 },
        output_tokens: if tokens_known { output } else { 0 },
        tokens_known,
        tools_proposed: proposed,
        tools_accepted: accepted,
        source: "cursor".into(),
        cost_complete: tokens_known,
        is_active: false,
    })
}

fn blake_short(s: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    format!("{:x}", h.finish())
}

