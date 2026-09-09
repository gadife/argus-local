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
    let since = Utc::now() - Duration::days(period_days as i64);
    let filtered: Vec<&SessionRecord> = sessions
        .iter()
        .filter(|s| s.started_at >= since)
        .collect();

    let sessions_n = filtered.len() as i64;
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
        .take(12)
        .map(|s| {
            let start = s.started_at.format("%H:%M").to_string();
            let end = s.ended_at.format("%H:%M").to_string();
            ActivityRow {
                time_range: format!("{start} - {end}"),
                tool: s.tool.clone(),
                model: s.model.clone(),
                tokens: s.total_tokens(),
                tokens_known: s.tokens_known,
                est_spend_usd: if s.cost_complete && s.tokens_known {
                    estimate_spend_usd(&s.model, s.input_tokens, s.output_tokens)
                } else {
                    0.0
                },
            }
        })
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
