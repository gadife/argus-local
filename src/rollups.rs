//! Period rollups + rough estimate pricing (NOT an invoice).
use crate::adapters::snapshot_for_period;
use crate::models::{
    ActivityRow, AdapterStatus, LanguageLock, PeriodRollup, SessionRecord, ShippingEvent,
    ShippingStub, ToolRollup,
};
use chrono::{Duration, Utc};
use std::collections::{HashMap, HashSet};

/// Rough public list-price heuristics - labeled estimate only.
pub fn estimate_spend_usd(model: &str, input: i64, output: i64) -> f64 {
    let m = model.to_lowercase();
    let (in_per_m, out_per_m) = if m.contains("opus") {
        (15.0, 75.0)
    } else if m.contains("sonnet") {
        (3.0, 15.0)
    } else if m.contains("haiku") {
        (0.80, 4.0)
    } else if m.contains("gpt-4.1") || m.contains("gpt-4o") {
        (2.50, 10.0)
    } else if m.contains("codex") || m.contains("o3") || m.contains("o4") {
        (2.0, 8.0)
    } else if m.contains("grok") {
        (3.0, 15.0)
    } else {
        (3.0, 12.0)
    };
    (input as f64 / 1_000_000.0) * in_per_m + (output as f64 / 1_000_000.0) * out_per_m
}

pub struct ShippingContext<'a> {
    pub configured: bool,
    pub events: &'a [ShippingEvent],
    pub auth_source: Option<&'a str>,
    pub login: Option<&'a str>,
}

pub fn rollup(
    sessions: &[SessionRecord],
    period_days: u32,
    adapter_status: Vec<AdapterStatus>,
    used_fixtures: bool,
    shipping: ShippingContext<'_>,
) -> PeriodRollup {
    let now = Utc::now();
    let since = now - Duration::days(period_days as i64);
    let mut filtered: Vec<&SessionRecord> = sessions
        .iter()
        .filter(|s| s.overlaps_period(since))
        .collect();
    filtered.sort_by(|a, b| {
        b.is_active
            .cmp(&a.is_active)
            .then_with(|| b.started_at.cmp(&a.started_at))
    });

    let sessions_n = filtered.len() as i64;
    let sessions_active = filtered.iter().filter(|s| s.is_active).count() as i64;
    let tokens: i64 = filtered.iter().map(|s| s.total_tokens()).sum();
    let any_tokens_known = filtered.iter().any(|s| s.tokens_known);
    let est_spend: f64 = filtered
        .iter()
        .map(|s| {
            if s.cost_complete && s.tokens_known {
                estimate_spend_usd(&s.model, s.input_tokens, s.output_tokens)
            } else {
                0.0
            }
        })
        .sum();

    let proposed: i64 = filtered.iter().map(|s| s.tools_proposed).sum();
    let accepted: i64 = filtered.iter().map(|s| s.tools_accepted).sum();
    let tool_accept_pct = if proposed > 0 {
        Some((accepted as f64 / proposed as f64) * 100.0)
    } else {
        None
    };

    let mut models = HashSet::new();
    for s in &filtered {
        models.insert(s.model.clone());
    }

    // tool -> (tokens, sessions, incomplete, tokens_known)
    let mut by_tool_map: HashMap<String, (i64, i64, bool, bool)> = HashMap::new();
    for s in &filtered {
        let e = by_tool_map
            .entry(s.tool.clone())
            .or_insert((0, 0, false, false));
        e.0 += s.total_tokens();
        e.1 += 1;
        if !s.cost_complete {
            e.2 = true;
        }
        if s.tokens_known {
            e.3 = true;
        }
    }

    let mut by_tool: Vec<ToolRollup> = by_tool_map
        .into_iter()
        .map(|(tool, (tok, sess, incomplete, known))| ToolRollup {
            tool,
            tokens: tok,
            tokens_known: known,
            sessions: sess,
            cost_incomplete: incomplete,
            share_pct: if tokens > 0 && known {
                (tok as f64 / tokens as f64) * 100.0
            } else if tokens > 0 {
                (tok as f64 / tokens as f64) * 100.0
            } else {
                0.0
            },
        })
        .collect();
    by_tool.sort_by(|a, b| b.tokens.cmp(&a.tokens));

    let activity: Vec<ActivityRow> = filtered
        .iter()
        .take(200)
        .map(|s| activity_row_at(s, &adapter_status, now))
        .collect();

    let shipping_snap = snapshot_for_period(
        shipping.configured,
        shipping.events,
        period_days,
        shipping.auth_source,
        shipping.login,
    );

    PeriodRollup {
        period_days,
        sessions: sessions_n,
        sessions_active,
        tokens,
        tokens_known: any_tokens_known,
        est_spend_usd: est_spend,
        tool_accept_pct,
        models_unique: models.len() as i64,
        by_tool,
        activity,
        shipping: shipping_snap,
        adapter_status,
        language_lock: LanguageLock::default(),
        used_fixtures,
    }
}

pub fn format_tokens(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Unconfigured shipping placeholder (tests / fixtures without GitHub).
#[allow(dead_code)]
pub fn absent_shipping() -> ShippingStub {
    snapshot_for_period(false, &[], 7, None, None)
}

/// Build a single ActivityRow for session detail API (ARG-38).
pub fn activity_row_for(s: &SessionRecord, adapter_status: &[AdapterStatus]) -> ActivityRow {
    activity_row_at(s, adapter_status, Utc::now())
}

fn activity_row_at(
    s: &SessionRecord,
    adapter_status: &[AdapterStatus],
    now: chrono::DateTime<Utc>,
) -> ActivityRow {
    let start = s.started_at.format("%Y-%m-%d %H:%M").to_string();
    let end_at = if s.is_active { now } else { s.ended_at };
    let end = end_at.format("%H:%M").to_string();
    let start_hm = s.started_at.format("%H:%M").to_string();
    let secs = (end_at - s.started_at).num_seconds().max(0);
    let duration = if secs >= 3600 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m", secs / 60)
    } else if secs > 0 {
        format!("{secs}s")
    } else {
        String::new()
    };
    let adapter_note = adapter_status
        .iter()
        .find(|a| {
            a.name.eq_ignore_ascii_case(&s.tool) || a.name.eq_ignore_ascii_case(&s.source)
        })
        .filter(|a| a.partial || !a.ok)
        .map(|a| a.detail.clone())
        .unwrap_or_default();
    let time_range = if s.is_active {
        format!("{start_hm} · active")
    } else {
        format!("{start_hm} - {end}")
    };
    ActivityRow {
        id: s.id.clone(),
        time_range,
        started_at: start,
        ended_at: if s.is_active {
            "now".into()
        } else {
            s.ended_at.format("%Y-%m-%d %H:%M").to_string()
        },
        duration,
        tool: s.tool.clone(),
        model: s.model.clone(),
        tokens: s.total_tokens(),
        input_tokens: if s.tokens_known { s.input_tokens } else { 0 },
        output_tokens: if s.tokens_known { s.output_tokens } else { 0 },
        tokens_known: s.tokens_known,
        tools_proposed: s.tools_proposed,
        tools_accepted: s.tools_accepted,
        cost_complete: s.cost_complete,
        source: s.source.clone(),
        adapter_note,
        est_spend_usd: if s.cost_complete && s.tokens_known {
            estimate_spend_usd(&s.model, s.input_tokens, s.output_tokens)
        } else {
            0.0
        },
        is_active: s.is_active,
    }
}

pub fn sessions_kpi_sub(sessions: i64, sessions_active: i64) -> String {
    if sessions_active > 0 {
        let completed = (sessions - sessions_active).max(0);
        format!("{completed} completed · {sessions_active} active")
    } else {
        "completed".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(
        id: &str,
        started_hours_ago: i64,
        ended_hours_ago: i64,
        active: bool,
    ) -> SessionRecord {
        let now = Utc::now();
        SessionRecord {
            id: id.into(),
            tool: "Claude Code".into(),
            model: "claude-sonnet-4".into(),
            started_at: now - Duration::hours(started_hours_ago),
            ended_at: now - Duration::hours(ended_hours_ago),
            input_tokens: 10,
            output_tokens: 2,
            tokens_known: true,
            tools_proposed: 0,
            tools_accepted: 0,
            source: "claude".into(),
            cost_complete: true,
            is_active: active,
        }
    }

    #[test]
    fn period_includes_active_session_that_started_before_window() {
        let sessions = vec![
            rec("old-done", 48, 40, false),
            rec("old-still-open", 48, 0, true),
            rec("today", 2, 1, false),
        ];
        let roll = rollup(&sessions, 1, vec![], false, ShippingContext {
            configured: false,
            events: &[],
            auth_source: None,
            login: None,
        });
        assert_eq!(roll.sessions, 2);
        assert_eq!(roll.sessions_active, 1);
        assert_eq!(roll.activity[0].id, "old-still-open");
        assert!(roll.activity[0].is_active);
        assert!(roll.activity[0].time_range.contains("active"));
        assert_eq!(sessions_kpi_sub(roll.sessions, roll.sessions_active), "1 completed · 1 active");
    }

    #[test]
    fn period_includes_session_that_ended_inside_window() {
        // Started 3 days ago, last event today — 1-day window still counts it.
        let sessions = vec![rec("span", 72, 1, false)];
        let roll = rollup(&sessions, 1, vec![], false, ShippingContext {
            configured: false,
            events: &[],
            auth_source: None,
            login: None,
        });
        assert_eq!(roll.sessions, 1);
        assert!(!roll.activity[0].is_active);
    }
}
