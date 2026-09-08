//! Claude Code JSONL adapter — best-effort ~/.claude/**/*.jsonl
use crate::models::{AdapterStatus, SessionRecord};
use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub fn claude_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".claude"));
        // Windows Claude Code sometimes under AppData
        if let Ok(appdata) = std::env::var("APPDATA") {
            roots.push(PathBuf::from(appdata).join("Claude").join(".claude"));
        }
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            roots.push(PathBuf::from(local).join("claude"));
        }
    }
    roots
}

pub fn scan_claude() -> Result<(Vec<SessionRecord>, AdapterStatus)> {
    let roots = claude_roots();
    let mut files: Vec<PathBuf> = Vec::new();
    for root in &roots {
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
            let p = entry.path();
            if p.is_file() {
                if let Some(ext) = p.extension() {
                    if ext == "jsonl" {
                        files.push(p.to_path_buf());
                    }
                }
            }
        }
    }

    if files.is_empty() {
        return Ok((
            vec![],
            AdapterStatus {
                name: "Claude Code".into(),
                ok: false,
                partial: true,
                detail: "no ~/.claude/**/*.jsonl found".into(),
            },
        ));
    }

    let mut by_session: HashMap<String, SessionAcc> = HashMap::new();
    for f in &files {
        if let Err(e) = ingest_jsonl(f, &mut by_session) {
            eprintln!("argus-local: claude skip {}: {e}", f.display());
        }
    }

    let sessions: Vec<SessionRecord> = by_session
        .into_iter()
        .map(|(id, acc)| acc.into_record(id))
        .collect();

    let n = sessions.len();
    Ok((
        sessions,
        AdapterStatus {
            name: "Claude Code".into(),
            ok: n > 0,
            partial: n == 0,
            detail: format!("{n} sessions from {} jsonl files", files.len()),
        },
    ))
}

struct SessionAcc {
    model: String,
    started: DateTime<Utc>,
    ended: DateTime<Utc>,
    input: i64,
    output: i64,
    proposed: i64,
    accepted: i64,
}

impl SessionAcc {
    fn into_record(self, id: String) -> SessionRecord {
        SessionRecord {
            id: format!("claude:{id}"),
            tool: "Claude Code".into(),
            model: if self.model.is_empty() {
                "Claude".into()
            } else {
                self.model
            },
            started_at: self.started,
            ended_at: self.ended,
            input_tokens: self.input,
            output_tokens: self.output,
            tools_proposed: self.proposed,
            tools_accepted: self.accepted,
            source: "claude".into(),
            cost_complete: true,
        }
    }
}

fn ingest_jsonl(path: &Path, map: &mut HashMap<String, SessionAcc>) -> Result<()> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let fallback_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

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
            .unwrap_or(&fallback_id)
            .to_string();

        let ts = parse_ts(&v).unwrap_or_else(Utc::now);
        let model = v
            .get("model")
            .and_then(|x| x.as_str())
            .or_else(|| v.pointer("/message/model").and_then(|x| x.as_str()))
            .unwrap_or("")
            .to_string();

        let (inp, out) = extract_tokens(&v);
        let (prop, acc) = extract_tools(&v);

        let entry = map.entry(sid).or_insert_with(|| SessionAcc {
            model: model.clone(),
            started: ts,
            ended: ts,
            input: 0,
            output: 0,
            proposed: 0,
            accepted: 0,
        });
        if !model.is_empty() {
            entry.model = model;
        }
        if ts < entry.started {
            entry.started = ts;
        }
        if ts > entry.ended {
            entry.ended = ts;
        }
        entry.input += inp;
        entry.output += out;
        entry.proposed += prop;
        entry.accepted += acc;
    }
    Ok(())
}

fn parse_ts(v: &Value) -> Option<DateTime<Utc>> {
    if let Some(s) = v.get("timestamp").and_then(|x| x.as_str()) {
        if let Ok(d) = DateTime::parse_from_rfc3339(s) {
            return Some(d.with_timezone(&Utc));
        }
    }
    if let Some(ms) = v
        .get("timestamp")
        .and_then(|x| x.as_i64())
        .or_else(|| v.get("ts").and_then(|x| x.as_i64()))
    {
        return Utc.timestamp_millis_opt(ms).single();
    }
    None
}

fn extract_tokens(v: &Value) -> (i64, i64) {
    let usage = v
        .get("usage")
        .or_else(|| v.pointer("/message/usage"))
        .cloned()
        .unwrap_or(Value::Null);
    let inp = usage
        .get("input_tokens")
        .or_else(|| usage.get("inputTokens"))
        .and_then(|x| x.as_i64())
        .unwrap_or(0);
    let out = usage
        .get("output_tokens")
        .or_else(|| usage.get("outputTokens"))
        .and_then(|x| x.as_i64())
        .unwrap_or(0);
    if inp + out > 0 {
        return (inp, out);
    }
    // Some Claude Code lines nest token counts differently
    let inp = v
        .pointer("/message/usage/input_tokens")
        .and_then(|x| x.as_i64())
        .unwrap_or(0);
    let out = v
        .pointer("/message/usage/output_tokens")
        .and_then(|x| x.as_i64())
        .unwrap_or(0);
    (inp, out)
}

fn extract_tools(v: &Value) -> (i64, i64) {
    let typ = v
        .get("type")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    // Heuristic: tool_use / tool_result events
    if typ.contains("tool_use") || v.pointer("/message/content").is_some() {
        if let Some(arr) = v.pointer("/message/content").and_then(|x| x.as_array()) {
            let mut prop = 0i64;
            let mut acc = 0i64;
            for item in arr {
                let t = item.get("type").and_then(|x| x.as_str()).unwrap_or("");
                if t == "tool_use" {
                    prop += 1;
                    acc += 1; // accepted by default when present in transcript
                }
            }
            return (prop, acc);
        }
    }
    if typ == "tool_use" {
        return (1, 1);
    }
    (0, 0)
}
