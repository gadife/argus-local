//! On-demand local prompt/response text extraction (opt-in only).
//! Never reads auth.json / tokens / PATs. Only fields that exist per adapter.
use anyhow::Result;
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// Soft cap so UI/API stays usable; UI still truncates further with Expand.
const MAX_FIELD_CHARS: usize = 24_000;

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct SessionTexts {
    pub prompt_text: Option<String>,
    pub response_text: Option<String>,
}

impl SessionTexts {
    pub fn is_empty(&self) -> bool {
        self.prompt_text
            .as_ref()
            .map(|s| s.trim().is_empty())
            .unwrap_or(true)
            && self
                .response_text
                .as_ref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(true)
    }
}

/// Fixed fixture session that carries sample prompt/response when opted in.
pub const FIXTURE_PROMPTS_ID: &str = "fixture:claude:arg37-prompts-demo";

/// Load prompt/response for a session id when available on disk / fixtures.
/// Returns empty texts (not inventing) when adapter has no fields.
pub fn load_session_texts(session_id: &str) -> Result<SessionTexts> {
    if session_id == FIXTURE_PROMPTS_ID {
        return Ok(fixture_texts());
    }
    if session_id.starts_with("fixture:") {
        return Ok(SessionTexts::default());
    }
    if let Some(rest) = session_id.strip_prefix("grok:") {
        return Ok(load_grok_texts(rest).unwrap_or_default());
    }
    if let Some(rest) = session_id.strip_prefix("claude:") {
        return Ok(load_claude_texts(rest).unwrap_or_default());
    }
    if session_id.starts_with("cursor:") {
        return Ok(load_cursor_texts(session_id).unwrap_or_default());
    }
    Ok(SessionTexts::default())
}

fn fixture_texts() -> SessionTexts {
    SessionTexts {
        prompt_text: Some(
            "Summarize yesterday's local adapter scan and list any sessions with incomplete cost."
                .into(),
        ),
        response_text: Some(
            "Scanned Claude Code, Cursor, and Grok locally. Two Grok sessions report context tokens only (cost incomplete). No raw prompts were included because prompts opt-in was Off."
                .into(),
        ),
    }
}

fn truncate_field(mut s: String) -> String {
    if s.chars().count() > MAX_FIELD_CHARS {
        s = s.chars().take(MAX_FIELD_CHARS).collect();
        s.push_str("\n...[truncated]");
    }
    s
}

fn nonempty(s: String) -> Option<String> {
    let t = s.trim().to_string();
    if t.is_empty() {
        None
    } else {
        Some(truncate_field(t))
    }
}

fn extract_user_query(text: &str) -> Option<String> {
    if let Some(start) = text.find("<user_query>") {
        let after = &text[start + "<user_query>".len()..];
        if let Some(end) = after.find("</user_query>") {
            return nonempty(after[..end].to_string());
        }
    }
    if text.contains("<system-reminder>") || text.contains("<user_info>") {
        return None;
    }
    nonempty(text.to_string())
}

fn content_to_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(arr) => {
            let mut parts = Vec::new();
            for item in arr {
                if let Some(t) = item.get("text").and_then(|x| x.as_str()) {
                    parts.push(t.to_string());
                } else if item.get("type").and_then(|x| x.as_str()) == Some("text") {
                    if let Some(tx) = item.get("text").and_then(|x| x.as_str()) {
                        parts.push(tx.to_string());
                    }
                }
            }
            parts.join("\n")
        }
        _ => String::new(),
    }
}

fn load_grok_texts(session_uuid: &str) -> Result<SessionTexts> {
    let Some(path) = find_grok_chat_history(session_uuid) else {
        return Ok(SessionTexts::default());
    };
    let file = File::open(&path)?;
    let reader = BufReader::new(file);
    let mut prompts: Vec<String> = Vec::new();
    let mut responses: Vec<String> = Vec::new();
    for line in reader.lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let typ = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
        if typ == "user" {
            let text = content_to_text(v.get("content").unwrap_or(&Value::Null));
            if let Some(q) = extract_user_query(&text) {
                prompts.push(q);
            }
        } else if typ == "assistant" {
            let text = content_to_text(v.get("content").unwrap_or(&Value::Null));
            if let Some(r) = nonempty(text) {
                responses.push(r);
            }
        }
    }
    Ok(SessionTexts {
        prompt_text: prompts.first().cloned(),
        response_text: responses.last().cloned(),
    })
}

fn find_grok_chat_history(session_uuid: &str) -> Option<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".grok").join("sessions"));
    }
    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        let p = PathBuf::from(userprofile).join(".grok").join("sessions");
        if !roots.iter().any(|r| r == &p) {
            roots.push(p);
        }
    }
    for root in roots {
        if !root.exists() {
            continue;
        }
        for entry in walkdir::WalkDir::new(&root)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.file_name().and_then(|s| s.to_str()) != Some("chat_history.jsonl") {
                continue;
            }
            let parent = path.parent()?.file_name()?.to_str()?;
            if parent == session_uuid {
                return Some(path.to_path_buf());
            }
        }
    }
    None
}

fn load_claude_texts(session_id: &str) -> Result<SessionTexts> {
    let roots = super::claude::claude_roots();
    let mut prompts: Vec<String> = Vec::new();
    let mut responses: Vec<String> = Vec::new();
    for root in roots {
        if !root.exists() {
            continue;
        }
        for entry in walkdir::WalkDir::new(&root)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                continue;
            }
            let _ = ingest_claude_file(path, session_id, &mut prompts, &mut responses);
        }
    }
    Ok(SessionTexts {
        prompt_text: prompts.first().cloned(),
        response_text: responses.last().cloned(),
    })
}

fn ingest_claude_file(
    path: &Path,
    want_sid: &str,
    prompts: &mut Vec<String>,
    responses: &mut Vec<String>,
) -> Result<()> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let fallback = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    for line in reader.lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let sid = v
            .get("sessionId")
            .or_else(|| v.get("session_id"))
            .and_then(|x| x.as_str())
            .unwrap_or(fallback);
        if sid != want_sid {
            continue;
        }
        let typ = v
            .get("type")
            .and_then(|x| x.as_str())
            .or_else(|| v.get("role").and_then(|x| x.as_str()))
            .or_else(|| v.pointer("/message/role").and_then(|x| x.as_str()))
            .unwrap_or("");
        let content = v
            .get("content")
            .or_else(|| v.pointer("/message/content"))
            .cloned()
            .unwrap_or(Value::Null);
        let text = content_to_text(&content);
        if typ == "user" || typ == "human" {
            if let Some(q) = extract_user_query(&text).or_else(|| nonempty(text.clone())) {
                if q.len() < 8_000 || prompts.is_empty() {
                    prompts.push(q);
                }
            }
        } else if typ == "assistant" || typ == "ai" {
            if let Some(r) = nonempty(text) {
                responses.push(r);
            }
        }
    }
    Ok(())
}

fn load_cursor_texts(session_id: &str) -> Result<SessionTexts> {
    // Re-walk Cursor DBs and match the same hashed id used at scan time.
    let files = super::cursor::find_vscdb_files_for_prompts();
    for db_path in files {
        if let Ok(texts) = cursor_texts_from_db(&db_path, session_id) {
            if !texts.is_empty() {
                return Ok(texts);
            }
        }
    }
    Ok(SessionTexts::default())
}

fn cursor_texts_from_db(path: &Path, want_id: &str) -> Result<SessionTexts> {
    let tmp = std::env::temp_dir().join(format!(
        "argus-cursor-prompts-{}.vscdb",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("db")
    ));
    std::fs::copy(path, &tmp)?;
    let conn = rusqlite::Connection::open(&tmp)?;
    let mut found = SessionTexts::default();

    if table_exists(&conn, "ItemTable") {
        let mut stmt = conn.prepare("SELECT key, value FROM ItemTable")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for r in rows.flatten() {
            let (key, value) = r;
            if let Some((id, texts)) = cursor_id_and_texts(&key, &value) {
                if id == want_id && !texts.is_empty() {
                    found = texts;
                    break;
                }
            }
        }
    }
    if found.is_empty() && table_exists(&conn, "cursorDiskKV") {
        let mut stmt = conn.prepare("SELECT key, value FROM cursorDiskKV")?;
        let rows = stmt.query_map([], |row| {
            let key: String = row.get(0)?;
            let value: Vec<u8> = row.get(1)?;
            Ok((key, String::from_utf8_lossy(&value).to_string()))
        })?;
        for r in rows.flatten() {
            let (key, value) = r;
            if let Some((id, texts)) = cursor_id_and_texts(&key, &value) {
                if id == want_id && !texts.is_empty() {
                    found = texts;
                    break;
                }
            }
        }
    }
    let _ = std::fs::remove_file(&tmp);
    Ok(found)
}

fn table_exists(conn: &rusqlite::Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        [name],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .unwrap_or(false)
}

fn cursor_id_and_texts(key: &str, value: &str) -> Option<(String, SessionTexts)> {
    let v: Value = serde_json::from_str(value).ok()?;
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
    let created_ms = v
        .get("createdAt")
        .or_else(|| v.get("timestamp"))
        .or_else(|| v.get("lastUpdatedAt"))
        .and_then(|x| x.as_i64())
        .unwrap_or(0);
    if created_ms == 0 && input + output == 0 && v.get("bubbles").is_none() && v.get("messages").is_none()
    {
        return None;
    }
    let id = format!(
        "cursor:{}",
        blake_short(&format!("{key}:{created_ms}:{input}:{output}"))
    );
    let texts = extract_cursor_message_texts(&v);
    Some((id, texts))
}

fn extract_cursor_message_texts(v: &Value) -> SessionTexts {
    let mut prompts = Vec::new();
    let mut responses = Vec::new();

    if let Some(arr) = v.get("bubbles").and_then(|x| x.as_array()) {
        for b in arr {
            let typ = b
                .get("type")
                .or_else(|| b.get("bubbleType"))
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_lowercase();
            let text = b
                .get("text")
                .or_else(|| b.get("rawText"))
                .or_else(|| b.pointer("/content"))
                .and_then(|x| {
                    if x.is_string() {
                        x.as_str().map(|s| s.to_string())
                    } else {
                        Some(content_to_text(x))
                    }
                })
                .unwrap_or_default();
            if text.trim().is_empty() {
                continue;
            }
            if typ.contains("user") || typ == "human" {
                if let Some(q) = nonempty(text) {
                    prompts.push(q);
                }
            } else if typ.contains("ai") || typ.contains("assistant") || typ == "bot" {
                if let Some(r) = nonempty(text) {
                    responses.push(r);
                }
            }
        }
    }

    if let Some(arr) = v.get("messages").and_then(|x| x.as_array()) {
        for m in arr {
            let role = m
                .get("role")
                .or_else(|| m.get("type"))
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_lowercase();
            let text = content_to_text(m.get("content").unwrap_or(&Value::Null));
            if text.trim().is_empty() {
                if let Some(t) = m.get("text").and_then(|x| x.as_str()) {
                    if role.contains("user") {
                        if let Some(q) = nonempty(t.to_string()) {
                            prompts.push(q);
                        }
                    } else if role.contains("assistant") || role.contains("ai") {
                        if let Some(r) = nonempty(t.to_string()) {
                            responses.push(r);
                        }
                    }
                }
                continue;
            }
            if role.contains("user") {
                if let Some(q) = nonempty(text) {
                    prompts.push(q);
                }
            } else if role.contains("assistant") || role.contains("ai") {
                if let Some(r) = nonempty(text) {
                    responses.push(r);
                }
            }
        }
    }

    SessionTexts {
        prompt_text: prompts.first().cloned(),
        response_text: responses.last().cloned(),
    }
}

fn blake_short(s: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    format!("{:x}", h.finish())
}
