//! Claude Code JSONL adapter — `~/.claude/projects/<project>/<session>.jsonl`
//!
//! One UUID file is one session. Nested subagent transcripts, `history.jsonl`,
//! and `timeline.jsonl` are not sessions.
//!
//! Active = `~/.claude/sessions/{pid}.json` still on disk (Claude removes it
//! when the process exits) OR last transcript event within 15 minutes.
use crate::models::{AdapterStatus, SessionRecord};
use anyhow::Result;
use chrono::{DateTime, Duration, TimeZone, Utc};
use serde_json::Value;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// How recently a transcript must have been written to count as still running.
const ACTIVE_WITHIN: Duration = Duration::minutes(15);

pub fn claude_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        let p = PathBuf::from(dir);
        if !p.as_os_str().is_empty() {
            roots.push(p);
        }
    }
    if let Some(home) = dirs::home_dir() {
        let home_claude = home.join(".claude");
        if !roots.iter().any(|r| r == &home_claude) {
            roots.push(home_claude);
        }
        // Windows Claude Code sometimes under AppData
        if let Ok(appdata) = std::env::var("APPDATA") {
            let p = PathBuf::from(appdata).join("Claude").join(".claude");
            if !roots.iter().any(|r| r == &p) {
                roots.push(p);
            }
        }
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let p = PathBuf::from(local).join("claude");
            if !roots.iter().any(|r| r == &p) {
                roots.push(p);
            }
        }
    }
    roots
}

pub fn scan_claude() -> Result<(Vec<SessionRecord>, AdapterStatus)> {
    scan_claude_at(&claude_roots(), Utc::now())
}

fn scan_claude_at(
    roots: &[PathBuf],
    now: DateTime<Utc>,
) -> Result<(Vec<SessionRecord>, AdapterStatus)> {
    let live = collect_live_session_ids(roots);
    let mut files: Vec<PathBuf> = Vec::new();
    for root in roots {
        collect_session_files(root, &mut files);
    }

    if files.is_empty() {
        return Ok((
            vec![],
            AdapterStatus {
                name: "Claude Code".into(),
                ok: false,
                partial: true,
                detail: "no ~/.claude/projects/**/*.jsonl found".into(),
            },
        ));
    }

    let mut sessions: Vec<SessionRecord> = Vec::new();
    for f in &files {
        match ingest_session_file(f, now, &live) {
            Ok(Some(rec)) => sessions.push(rec),
            Ok(None) => {}
            Err(e) => eprintln!("argus-local: claude skip {}: {e}", f.display()),
        }
    }

    let n = sessions.len();
    let active = sessions.iter().filter(|s| s.is_active).count();
    let detail = if active > 0 {
        format!(
            "{n} sessions ({active} active) from {} jsonl files",
            files.len()
        )
    } else {
        format!("{n} sessions from {} jsonl files", files.len())
    };
    Ok((
        sessions,
        AdapterStatus {
            name: "Claude Code".into(),
            ok: n > 0,
            partial: n == 0,
            detail,
        },
    ))
}

/// Direct children of `projects/<project>/` that are session transcripts.
/// Skips `history.jsonl`, subagents, orphaned/superseded copies.
fn collect_session_files(root: &Path, out: &mut Vec<PathBuf>) {
    let projects = root.join("projects");
    let Ok(projs) = std::fs::read_dir(&projects) else {
        return;
    };
    for proj in projs.flatten() {
        let path = proj.path();
        if !path.is_dir() {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&path) else {
            continue;
        };
        for f in files.flatten() {
            let p = f.path();
            if is_session_jsonl(&p) {
                out.push(p);
            }
        }
    }
}

fn is_session_jsonl(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let name = match path.file_name().and_then(|s| s.to_str()) {
        Some(n) => n,
        None => return false,
    };
    if !name.ends_with(".jsonl") {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    if lower.contains("orphaned") || lower.contains("superseded") {
        return false;
    }
    // Skip metadata dumps that sit next to transcripts (timeline.jsonl, etc.)
    if lower == "timeline.jsonl" || lower == "history.jsonl" {
        return false;
    }
    is_session_uuid_name(name)
}

fn is_session_uuid_name(name: &str) -> bool {
    let stem = name.strip_suffix(".jsonl").unwrap_or(name);
    let parts: Vec<&str> = stem.split('-').collect();
    parts.len() == 5
        && parts[0].len() == 8
        && parts[1].len() == 4
        && parts[2].len() == 4
        && parts[3].len() == 4
        && parts[4].len() == 12
        && stem.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// Claude Code writes `~/.claude/sessions/{pid}.json` while a process is live
/// and deletes it on exit. `sessionId` is the transcript UUID.
fn collect_live_session_ids(roots: &[PathBuf]) -> HashSet<String> {
    let mut ids = HashSet::new();
    for root in roots {
        let dir = root.join("sessions");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if stem.is_empty() || !stem.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<Value>(&raw) else {
                continue;
            };
            if let Some(sid) = v.get("sessionId").and_then(|x| x.as_str()) {
                if !sid.is_empty() {
                    ids.insert(sid.to_string());
                }
            }
        }
    }
    ids
}

struct SessionAcc {
    model: String,
    started: Option<DateTime<Utc>>,
    ended: Option<DateTime<Utc>>,
    input: i64,
    output: i64,
    proposed: i64,
    accepted: i64,
    saw_line: bool,
    saw_conversation: bool,
}

impl SessionAcc {
    fn into_record(self, id: String, is_active: bool, file_mtime: DateTime<Utc>) -> SessionRecord {
        let started = self.started.unwrap_or(file_mtime);
        let mut ended = self.ended.unwrap_or(file_mtime);
        if ended < started {
            ended = started;
        }
        let tokens_known = self.input + self.output > 0;
        SessionRecord {
            id: format!("claude:{id}"),
            tool: "Claude Code".into(),
            model: if self.model.is_empty() {
                "Claude".into()
            } else {
                self.model
            },
            started_at: started,
            ended_at: ended,
            input_tokens: self.input,
            output_tokens: self.output,
            tokens_known,
            tools_proposed: self.proposed,
            tools_accepted: self.accepted,
            source: "claude".into(),
            cost_complete: tokens_known,
            is_active,
        }
    }
}

fn ingest_session_file(
    path: &Path,
    now: DateTime<Utc>,
    live: &HashSet<String>,
) -> Result<Option<SessionRecord>> {
    let file_mtime = mtime_utc(path).unwrap_or(now);
    let fallback_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut acc = SessionAcc {
        model: String::new(),
        started: None,
        ended: None,
        input: 0,
        output: 0,
        proposed: 0,
        accepted: 0,
        saw_line: false,
        saw_conversation: false,
    };

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
        acc.saw_line = true;

        if let Some(ts) = parse_ts(&v) {
            acc.started = Some(match acc.started {
                Some(s) if s <= ts => s,
                _ => ts,
            });
            acc.ended = Some(match acc.ended {
                Some(e) if e >= ts => e,
                _ => ts,
            });
        }

        let model = v
            .get("model")
            .and_then(|x| x.as_str())
            .or_else(|| v.pointer("/message/model").and_then(|x| x.as_str()))
            .unwrap_or("");
        if !model.is_empty() {
            acc.model = model.to_string();
        }

        let typ = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
        if typ == "user" || typ == "assistant" {
            acc.saw_conversation = true;
        }
        // Assistant lines carry per-turn usage. Skip progress (nested, would double-count)
        // and untimestamped metadata that used to bump ended_at to "now".
        if typ == "assistant" {
            let (inp, out) = extract_tokens(&v);
            acc.input += inp;
            acc.output += out;
        }
        let (prop, acc_tools) = extract_tools(&v);
        acc.proposed += prop;
        acc.accepted += acc_tools;
    }

    let recent_event = acc
        .ended
        .map(|e| now.signed_duration_since(e) <= ACTIVE_WITHIN)
        .unwrap_or(false);
    let recent_file = now.signed_duration_since(file_mtime) <= ACTIVE_WITHIN;
    let is_active = live.contains(&fallback_id) || recent_event || (acc.ended.is_none() && recent_file);

    if !acc.saw_conversation && !is_active {
        return Ok(None);
    }
    if !acc.saw_line && !is_active {
        return Ok(None);
    }

    Ok(Some(acc.into_record(fallback_id, is_active, file_mtime)))
}

fn mtime_utc(path: &Path) -> Option<DateTime<Utc>> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(DateTime::<Utc>::from(modified))
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
    let typ = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("argus-claude-{nanos}"));
        fs::create_dir_all(dir.join("projects").join("-Users-me-code")).unwrap();
        dir
    }

    fn write_jsonl(path: &Path, lines: &[&str]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = File::create(path).unwrap();
        for line in lines {
            writeln!(f, "{line}").unwrap();
        }
    }

    fn set_mtime(path: &Path, ts: DateTime<Utc>) {
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(ts.into())
            .unwrap();
    }

    #[test]
    fn scans_project_session_and_skips_history_and_subagents() {
        let root = temp_root();
        let proj = root.join("projects").join("-Users-me-code");
        let sid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        write_jsonl(
            &proj.join(format!("{sid}.jsonl")),
            &[
                r#"{"type":"user","timestamp":"2026-09-01T10:00:00Z","sessionId":"aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee","message":{"role":"user","content":"hi"}}"#,
                r#"{"type":"assistant","timestamp":"2026-09-01T10:05:00Z","message":{"model":"claude-sonnet-4","usage":{"input_tokens":100,"output_tokens":20}}}"#,
            ],
        );
        write_jsonl(
            &root.join("history.jsonl"),
            &[r#"{"type":"user","timestamp":"2026-09-01T12:00:00Z","sessionId":"history-not-a-session"}"#],
        );
        write_jsonl(
            &proj.join("timeline.jsonl"),
            &[r#"{"type":"user","timestamp":"2026-09-10T21:10:00Z","sessionId":"timeline"}"#],
        );
        write_jsonl(
            &proj.join(sid).join("subagents").join("agent-1.jsonl"),
            &[r#"{"type":"assistant","timestamp":"2026-09-01T10:06:00Z","message":{"model":"claude-haiku","usage":{"input_tokens":9,"output_tokens":1}}}"#],
        );
        let ended = DateTime::parse_from_rfc3339("2026-09-01T10:05:00Z")
            .unwrap()
            .with_timezone(&Utc);
        set_mtime(&proj.join(format!("{sid}.jsonl")), ended);

        let now = DateTime::parse_from_rfc3339("2026-09-10T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let (sessions, status) = scan_claude_at(&[root.clone()], now).unwrap();
        let _ = fs::remove_dir_all(&root);

        assert_eq!(sessions.len(), 1, "detail={}", status.detail);
        assert_eq!(sessions[0].id, format!("claude:{sid}"));
        assert_eq!(sessions[0].model, "claude-sonnet-4");
        assert_eq!(sessions[0].input_tokens, 100);
        assert_eq!(sessions[0].output_tokens, 20);
        assert!(!sessions[0].is_active);
        assert_eq!(
            sessions[0].started_at,
            DateTime::parse_from_rfc3339("2026-09-01T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        );
        assert_eq!(
            sessions[0].ended_at,
            DateTime::parse_from_rfc3339("2026-09-01T10:05:00Z")
                .unwrap()
                .with_timezone(&Utc)
        );
    }

    #[test]
    fn missing_timestamps_do_not_become_now() {
        let root = temp_root();
        let proj = root.join("projects").join("-Users-me-code");
        let sid = "bbbbbbbb-bbbb-cccc-dddd-eeeeeeeeeeee";
        write_jsonl(
            &proj.join(format!("{sid}.jsonl")),
            &[
                r#"{"type":"user","timestamp":"2026-09-01T10:00:00Z"}"#,
                r#"{"type":"file-history-snapshot"}"#,
                r#"{"type":"assistant","timestamp":"2026-09-01T10:02:00Z","message":{"model":"claude-opus-4","usage":{"input_tokens":10,"output_tokens":2}}}"#,
            ],
        );
        let ended = DateTime::parse_from_rfc3339("2026-09-01T10:02:00Z")
            .unwrap()
            .with_timezone(&Utc);
        set_mtime(&proj.join(format!("{sid}.jsonl")), ended);
        let now = DateTime::parse_from_rfc3339("2026-09-10T20:33:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let (sessions, _) = scan_claude_at(&[root.clone()], now).unwrap();
        let _ = fs::remove_dir_all(&root);
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].ended_at,
            DateTime::parse_from_rfc3339("2026-09-01T10:02:00Z")
                .unwrap()
                .with_timezone(&Utc)
        );
        assert_ne!(sessions[0].ended_at, now);
        assert!(!sessions[0].is_active);
    }

    #[test]
    fn recent_event_marks_session_active() {
        let root = temp_root();
        let proj = root.join("projects").join("-Users-me-code");
        let sid = "cccccccc-bbbb-cccc-dddd-eeeeeeeeeeee";
        let now = Utc::now();
        let last = (now - Duration::minutes(2)).to_rfc3339();
        write_jsonl(
            &proj.join(format!("{sid}.jsonl")),
            &[
                r#"{"type":"user","timestamp":"2026-08-01T08:00:00Z"}"#,
                &format!(
                    r#"{{"type":"assistant","timestamp":"{last}","message":{{"model":"claude-sonnet-4","usage":{{"input_tokens":50,"output_tokens":10}}}}}}"#
                ),
            ],
        );
        let (sessions, status) = scan_claude_at(&[root.clone()], now).unwrap();
        let _ = fs::remove_dir_all(&root);
        assert_eq!(sessions.len(), 1, "{}", status.detail);
        assert!(sessions[0].is_active, "expected active: {}", status.detail);
        assert!(status.detail.contains("active"));
    }

    #[test]
    fn live_pid_file_marks_idle_session_active() {
        let root = temp_root();
        let proj = root.join("projects").join("-Users-me-code");
        let sid = "eeeeeeee-bbbb-cccc-dddd-eeeeeeeeeeee";
        write_jsonl(
            &proj.join(format!("{sid}.jsonl")),
            &[
                r#"{"type":"user","timestamp":"2026-08-01T08:00:00Z"}"#,
                r#"{"type":"assistant","timestamp":"2026-08-01T08:30:00Z","message":{"model":"claude-sonnet-4","usage":{"input_tokens":50,"output_tokens":10}}}"#,
            ],
        );
        let ended = DateTime::parse_from_rfc3339("2026-08-01T08:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        set_mtime(&proj.join(format!("{sid}.jsonl")), ended);
        fs::create_dir_all(root.join("sessions")).unwrap();
        fs::write(
            root.join("sessions").join("4242.json"),
            format!(r#"{{"pid":4242,"sessionId":"{sid}","cwd":"/Users/me/code"}}"#),
        )
        .unwrap();
        let now = DateTime::parse_from_rfc3339("2026-09-10T21:10:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let (sessions, status) = scan_claude_at(&[root.clone()], now).unwrap();
        let _ = fs::remove_dir_all(&root);
        assert_eq!(sessions.len(), 1, "{}", status.detail);
        assert!(
            sessions[0].is_active,
            "open Claude process should stay active even when idle: {}",
            status.detail
        );
    }

    #[test]
    fn progress_usage_is_not_double_counted() {
        let root = temp_root();
        let proj = root.join("projects").join("-Users-me-code");
        let sid = "dddddddd-bbbb-cccc-dddd-eeeeeeeeeeee";
        write_jsonl(
            &proj.join(format!("{sid}.jsonl")),
            &[
                r#"{"type":"progress","message":{"usage":{"input_tokens":50,"output_tokens":10}}}"#,
                r#"{"type":"assistant","timestamp":"2026-09-01T10:00:00Z","message":{"model":"claude-sonnet-4","usage":{"input_tokens":50,"output_tokens":10}}}"#,
            ],
        );
        let ended = DateTime::parse_from_rfc3339("2026-09-01T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        set_mtime(&proj.join(format!("{sid}.jsonl")), ended);
        let now = DateTime::parse_from_rfc3339("2026-09-10T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let (sessions, _) = scan_claude_at(&[root.clone()], now).unwrap();
        let _ = fs::remove_dir_all(&root);
        assert_eq!(sessions[0].input_tokens, 50);
        assert_eq!(sessions[0].output_tokens, 10);
    }
}
