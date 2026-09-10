//! Session detail telemetry loader (ARG-38).
//! Reads on-disk signals / turn_completed usage / summary — never invents numbers,
//! never reads auth.json, prompt_context.json, or raw prompts.
use super::grok::grok_roots;
use crate::models::SessionTelemetry;
use anyhow::Result;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Load telemetry for a session id like `grok:{uuid}`, `claude:…`, `cursor:…`.
/// Missing fields stay None. Never invents counts.
pub fn load_session_telemetry(id: &str) -> Result<SessionTelemetry> {
    let (tool, raw_id) = split_id(id);
    let mut tel = SessionTelemetry {
        id: id.to_string(),
        tool: tool.to_string(),
        ..Default::default()
    };

    match tool {
        "grok" => fill_grok(&mut tel, raw_id)?,
        "claude" => fill_from_activity_equivalents(&mut tel, "Claude"),
        "cursor" => fill_from_activity_equivalents(&mut tel, "Cursor"),
        _ => {}
    }
    Ok(tel)
}

fn split_id(id: &str) -> (&str, &str) {
    if let Some((prefix, rest)) = id.split_once(':') {
        (prefix, rest)
    } else {
        ("unknown", id)
    }
}

fn fill_from_activity_equivalents(tel: &mut SessionTelemetry, label: &str) {
    // Claude/Cursor billable I/O already live on ActivityRow; detail API merges
    // those via server. On-disk deep telemetry for those tools is best-effort later.
    tel.context_note = Some(format!(
        "{label}: context window vs billable I/O mapped when present; estimates \u{2260} invoice"
    ));
}

fn fill_grok(tel: &mut SessionTelemetry, uuid: &str) -> Result<()> {
    let Some(dir) = find_grok_session_dir(uuid) else {
        tel.context_note = Some(
            "Grok session dir not found on disk — showing activity aggregates only".into(),
        );
        return Ok(());
    };

    // --- signals.json (context + signals extras) ---
    let signals_path = dir.join("signals.json");
    if signals_path.exists() {
        if let Ok(raw) = fs::read_to_string(&signals_path) {
            if let Ok(v) = serde_json::from_str::<Value>(&raw) {
                apply_signals(tel, &v);
            }
        }
    }

    // --- updates.jsonl turn_completed.usage (SUM) ---
    let updates_path = dir.join("updates.jsonl");
    if updates_path.exists() {
        if let Ok(raw) = fs::read_to_string(&updates_path) {
            apply_updates_usage(tel, &raw);
        }
    }

    // --- summary.json (title / kind / cwd) — never prompt_context / auth ---
    let summary_path = dir.join("summary.json");
    if summary_path.exists() {
        if let Ok(raw) = fs::read_to_string(&summary_path) {
            if let Ok(v) = serde_json::from_str::<Value>(&raw) {
                apply_summary(tel, &v);
            }
        }
    }

    // Honesty labels
    if tel.context_tokens_used.is_some() {
        tel.context_note = Some(
            "Context = window fill (not billable in/out). Billable I/O from turn usage when present."
                .into(),
        );
    }
    if tel.cost_usd_estimate.is_some() {
        tel.cost_note = Some("estimate \u{2260} invoice".into());
    }

    Ok(())
}

fn find_grok_session_dir(uuid: &str) -> Option<PathBuf> {
    for root in grok_roots() {
        let sessions = root.join("sessions");
        if !sessions.is_dir() {
            continue;
        }
        for entry in WalkDir::new(&sessions).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() && path.file_name().and_then(|s| s.to_str()) == Some(uuid) {
                // Prefer dirs that actually have signals.json
                if path.join("signals.json").exists() || path.join("summary.json").exists() {
                    return Some(path.to_path_buf());
                }
            }
        }
    }
    None
}

fn apply_signals(tel: &mut SessionTelemetry, v: &Value) {
    tel.context_tokens_used = i64_field(v, "contextTokensUsed");
    tel.context_window_tokens = i64_field(v, "contextWindowTokens");
    if let (Some(used), Some(window)) = (tel.context_tokens_used, tel.context_window_tokens) {
        if window > 0 {
            tel.context_pct = Some((used as f64) * 100.0 / (window as f64));
        }
    }

    tel.turn_count = i64_field(v, "turnCount");
    tel.tool_call_count = i64_field(v, "toolCallCount");
    tel.session_duration_seconds = i64_field(v, "sessionDurationSeconds");
    tel.avg_time_to_first_token_ms = f64_field(v, "avgTimeToFirstTokenMs");
    tel.avg_response_time_ms = f64_field(v, "avgResponseTimeMs");

    if let Some(m) = v.get("primaryModelId").and_then(|x| x.as_str()) {
        if !m.is_empty() {
            tel.model = Some(m.to_string());
        }
    }

    // toolsUsed may be object map {name: count} or array
    tel.tools_used = parse_tools_used(v.get("toolsUsed"));
}

fn parse_tools_used(v: Option<&Value>) -> Option<Vec<String>> {
    let v = v?;
    if let Some(arr) = v.as_array() {
        let names: Vec<String> = arr
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect();
        return if names.is_empty() { None } else { Some(names) };
    }
    if let Some(obj) = v.as_object() {
        let mut names: Vec<String> = obj.keys().cloned().collect();
        names.sort();
        return if names.is_empty() { None } else { Some(names) };
    }
    None
}

fn apply_updates_usage(tel: &mut SessionTelemetry, raw: &str) {
    let mut input: i64 = 0;
    let mut output: i64 = 0;
    let mut total: i64 = 0;
    let mut cached_read: i64 = 0;
    let mut cache_creation: i64 = 0;
    let mut reasoning: i64 = 0;
    let mut model_calls: i64 = 0;
    let mut api_ms: i64 = 0;
    let mut cost_ticks: i64 = 0;
    let mut turns_with_usage = 0i64;

    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let update = v
            .pointer("/params/update")
            .or_else(|| v.get("update"))
            .unwrap_or(&v);
        let kind = update
            .get("sessionUpdate")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        if kind != "turn_completed" {
            continue;
        }
        let Some(usage) = update.get("usage") else {
            continue;
        };
        turns_with_usage += 1;
        input += i64_field(usage, "inputTokens").unwrap_or(0);
        output += i64_field(usage, "outputTokens").unwrap_or(0);
        total += i64_field(usage, "totalTokens").unwrap_or(0);
        cached_read += i64_field(usage, "cachedReadTokens")
            .or_else(|| i64_field(usage, "cache_read_input_tokens"))
            .unwrap_or(0);
        cache_creation += i64_field(usage, "cacheCreationTokens")
            .or_else(|| i64_field(usage, "cache_creation_input_tokens"))
            .unwrap_or(0);
        reasoning += i64_field(usage, "reasoningTokens").unwrap_or(0);
        model_calls += i64_field(usage, "modelCalls").unwrap_or(0);
        api_ms += i64_field(usage, "apiDurationMs").unwrap_or(0);
        cost_ticks += i64_field(usage, "costUsdTicks").unwrap_or(0);
    }

    if turns_with_usage == 0 {
        return;
    }

    // Only set fields that were actually observed (non-zero OR present across turns).
    // We set them because turn_completed.usage existed — zeros are real observations.
    tel.input_tokens = Some(input);
    tel.output_tokens = Some(output);
    tel.total_tokens = Some(total);
    tel.cached_read_tokens = Some(cached_read);
    tel.cache_creation_tokens = Some(cache_creation);
    tel.reasoning_tokens = Some(reasoning);
    tel.model_calls = Some(model_calls);
    tel.api_duration_ms = Some(api_ms);
    if cost_ticks > 0 {
        // Grok costUsdTicks appear to be micro-units (ticks / 1e9 ≈ USD observed live).
        // Keep as estimate only — label estimate ≠ invoice.
        tel.cost_usd_estimate = Some((cost_ticks as f64) / 1_000_000_000.0);
    }
}

fn apply_summary(tel: &mut SessionTelemetry, v: &Value) {
    if let Some(t) = v
        .get("generated_title")
        .or_else(|| v.get("session_summary"))
        .and_then(|x| x.as_str())
    {
        if !t.is_empty() {
            tel.generated_title = Some(t.to_string());
        }
    }
    if let Some(k) = v.get("session_kind").and_then(|x| x.as_str()) {
        if !k.is_empty() {
            tel.session_kind = Some(k.to_string());
        }
    }
    if let Some(cwd) = v
        .pointer("/info/cwd")
        .or_else(|| v.get("cwd"))
        .and_then(|x| x.as_str())
    {
        if !cwd.is_empty() {
            tel.cwd = Some(cwd.to_string());
        }
    }
    if tel.model.is_none() {
        if let Some(m) = v.get("current_model_id").and_then(|x| x.as_str()) {
            if !m.is_empty() {
                tel.model = Some(m.to_string());
            }
        }
    }
}

fn i64_field(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(|x| {
        x.as_i64()
            .or_else(|| x.as_u64().map(|u| u as i64))
            .or_else(|| x.as_f64().map(|f| f as i64))
    })
}

fn f64_field(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(|x| {
        x.as_f64()
            .or_else(|| x.as_i64().map(|i| i as f64))
            .or_else(|| x.as_u64().map(|u| u as f64))
    })
}

#[allow(dead_code)]
fn _path_exists(p: &Path) -> bool {
    p.exists()
}