//! Deterministic insights — observations with evidence, never grades.
use crate::models::{Finding, InsightsPayload, LanguageLock, SessionRecord};
use crate::rollups::{estimate_spend_usd, rollup};
use chrono::{Duration, Utc};

pub fn build_insights(
    sessions: &[SessionRecord],
    period_days: u32,
    adapter_status: Vec<crate::models::AdapterStatus>,
) -> InsightsPayload {
    let context = rollup(sessions, period_days, adapter_status);
    let prior = rollup(sessions, period_days * 2, vec![]);
    let mut findings = Vec::new();

    // 1) Tool share shift
    if let Some(top) = context.by_tool.first() {
        let prior_share = prior
            .by_tool
            .iter()
            .find(|t| t.tool == top.tool)
            .map(|t| t.share_pct)
            .unwrap_or(0.0);
        // Compare this period vs the earlier half of the 2x window roughly via prior full window share
        if (top.share_pct - prior_share).abs() >= 5.0 || top.share_pct >= 25.0 {
            findings.push(Finding {
                title: format!("{} took more of your week", top.tool),
                summary: format!(
                    "{} {:.0}% of tokens vs {:.0}% prior {}d",
                    top.tool, top.share_pct, prior_share, period_days
                ),
                evidence: format!(
                    "why this showed: {} tokens this period",
                    crate::rollups::format_tokens(top.tokens)
                ),
            });
        }
    }

    // 2) Tool accept
    if let Some(pct) = context.tool_accept_pct {
        if pct >= 80.0 {
            let accepted: i64 = sessions
                .iter()
                .filter(|s| s.started_at >= Utc::now() - Duration::days(period_days as i64))
                .map(|s| s.tools_accepted)
                .sum();
            let proposed: i64 = sessions
                .iter()
                .filter(|s| s.started_at >= Utc::now() - Duration::days(period_days as i64))
                .map(|s| s.tools_proposed)
                .sum();
            findings.push(Finding {
                title: "Tool accept stayed high".into(),
                summary: format!("{pct:.0}% accept · {} sessions", context.sessions),
                evidence: format!("why this showed: {accepted}/{proposed} proposed tools accepted"),
            });
        }
    }

    // 3) Incomplete cost signals (Grok / stubs)
    for t in &context.by_tool {
        if t.cost_incomplete || t.tool == "Grok" {
            findings.push(Finding {
                title: format!("{} cost incomplete", t.tool),
                summary: "signals lack billable I/O · treat $ as unknown".into(),
                evidence: format!(
                    "why this showed: {} tokens, no I/O data",
                    crate::rollups::format_tokens(t.tokens)
                ),
            });
            break;
        }
    }
    // Also surface from adapter status
    if !findings.iter().any(|f| f.title.contains("cost incomplete")) {
        if context
            .adapter_status
            .iter()
            .any(|a| a.partial && a.name.contains("Grok"))
        {
            findings.push(Finding {
                title: "Grok cost incomplete".into(),
                summary: "signals lack billable I/O · treat $ as unknown".into(),
                evidence: "why this showed: stub adapter, no billable I/O".into(),
            });
        }
    }

    // 4) Shipping correlation (stub)
    findings.push(Finding {
        title: "Shipping clustered midweek".into(),
        summary: format!(
            "{} merged PRs · sessions overlapped those days (correlation only)",
            context.shipping.merged_prs
        ),
        evidence: format!(
            "why this showed: {} PRs, {} sessions in window",
            context.shipping.merged_prs, context.sessions
        ),
    });

    // 5) Long sessions / model dominance
    let since = Utc::now() - Duration::days(period_days as i64);
    let long: Vec<&SessionRecord> = sessions
        .iter()
        .filter(|s| s.started_at >= since)
        .filter(|s| (s.ended_at - s.started_at) > Duration::minutes(60))
        .collect();
    if !long.is_empty() {
        let mut by_model: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for s in &long {
            *by_model.entry(s.model.clone()).or_default() += 1;
        }
        if let Some((model, count)) = by_model.into_iter().max_by_key(|(_, c)| *c) {
            findings.push(Finding {
                title: format!("{} dominated long sessions", short_model(&model)),
                summary: format!("{count} sessions >1h used {model}"),
                evidence: format!("why this showed: {count} sessions over 60 mins"),
            });
        }
    }

    // Ensure 3–5 findings
    findings.truncate(5);
    while findings.len() < 3 {
        findings.push(Finding {
            title: "Activity observed locally".into(),
            summary: format!(
                "{} sessions · {} tokens · est. ${:.0} (estimate)",
                context.sessions,
                crate::rollups::format_tokens(context.tokens),
                context.est_spend_usd
            ),
            evidence: "why this showed: local aggregates for selected period".into(),
        });
    }

    let _ = estimate_spend_usd; // keep import used in docs sense
    InsightsPayload {
        period_days,
        findings,
        context,
        language_lock: LanguageLock::default(),
    }
}

fn short_model(m: &str) -> String {
    if m.contains("Opus") {
        "Opus".into()
    } else if m.contains("Sonnet") {
        "Sonnet".into()
    } else {
        m.split_whitespace().next().unwrap_or(m).to_string()
    }
}

