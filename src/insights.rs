//! Deterministic insights - observations with evidence, never grades.
use crate::models::{Finding, InsightsPayload, LanguageLock, SessionRecord};
use crate::rollups::{estimate_spend_usd, rollup, ShippingContext};
use chrono::{Duration, Utc};

pub fn build_insights(
    sessions: &[SessionRecord],
    period_days: u32,
    adapter_status: Vec<crate::models::AdapterStatus>,
    used_fixtures: bool,
    shipping: ShippingContext<'_>,
) -> InsightsPayload {
    let context = rollup(
        sessions,
        period_days,
        adapter_status,
        used_fixtures,
        ShippingContext {
            configured: shipping.configured,
            events: shipping.events,
            auth_source: shipping.auth_source,
            login: shipping.login,
        },
    );
    let prior = rollup(
        sessions,
        period_days * 2,
        vec![],
        used_fixtures,
        ShippingContext {
            configured: shipping.configured,
            events: shipping.events,
            auth_source: shipping.auth_source,
            login: shipping.login,
        },
    );
    let mut findings = Vec::new();

    // 1) Tool share shift — only when share materially INCREASED (not flat / down).
    if let Some(top) = context.by_tool.first() {
        let prior_share = prior
            .by_tool
            .iter()
            .find(|t| t.tool == top.tool)
            .map(|t| t.share_pct)
            .unwrap_or(0.0);
        let delta = top.share_pct - prior_share;
        if delta >= 5.0 {
            findings.push(Finding {
                title: format!("{} took more of your week", top.tool),
                summary: format!(
                    "{} {:.0}% of tokens vs {:.0}% prior {}d (+{:.0}pp)",
                    top.tool, top.share_pct, prior_share, period_days, delta
                ),
                evidence: format!(
                    "why this showed: {} tokens this period, share up {:.0}pp",
                    crate::rollups::format_tokens(top.tokens),
                    delta
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
                summary: format!("{pct:.0}% accept \u{2014} {} sessions", context.sessions),
                evidence: format!("why this showed: {accepted}/{proposed} proposed tools accepted"),
            });
        }
    }

    // 3) Incomplete cost signals (e.g. Grok context tokens only)
    for t in &context.by_tool {
        if t.cost_incomplete || t.tool == "Grok" {
            findings.push(Finding {
                title: format!("{} cost incomplete", t.tool),
                summary: "signals lack billable I/O \u{2014} treat $ as unknown".into(),
                evidence: format!(
                    "why this showed: {} tokens, no I/O data",
                    if t.tokens_known {
                        crate::rollups::format_tokens(t.tokens)
                    } else {
                        "unknown".into()
                    }
                ),
            });
            break;
        }
    }
    if !findings.iter().any(|f| f.title.contains("cost incomplete")) {
        if context
            .adapter_status
            .iter()
            .any(|a| a.partial && a.name.contains("Grok"))
        {
            findings.push(Finding {
                title: "Grok cost incomplete".into(),
                summary: "signals lack billable I/O \u{2014} treat $ as unknown".into(),
                evidence: "why this showed: local Grok signals lack billable I/O".into(),
            });
        }
    }

    // 4) Shipping — unconfigured vs observed correlation (never invent counts)
    if context.shipping.is_stub || !context.shipping.configured {
        findings.push(Finding {
            title: "Shipping not configured".into(),
            summary: "Opt-in GitHub shipping is off \u{2014} no observed PR/commit counts".into(),
            evidence: "why this showed: GitHub adapter not opted in (Settings / ARGUS_GITHUB_TOKEN / ARGUS_GITHUB_ENABLED)".into(),
        });
    } else {
        let prs = context.shipping.merged_prs.unwrap_or(0);
        findings.push(Finding {
            title: "Shipping alongside sessions".into(),
            summary: format!(
                "{prs} merged PRs \u{2014} sessions overlapped the window (correlation only, not a score)"
            ),
            evidence: format!(
                "why this showed: {prs} merged PRs, {} commits, {} files touched, {} sessions in window",
                context.shipping.commits.unwrap_or(0),
                context.shipping.files_touched.unwrap_or(0),
                context.sessions
            ),
        });
    }

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

    findings.truncate(5);
    while findings.len() < 3 {
        findings.push(Finding {
            title: "Activity observed locally".into(),
            summary: format!(
                "{} sessions \u{2014} {} tokens \u{2014} est. ${:.0} (estimate)",
                context.sessions,
                if context.tokens_known {
                    crate::rollups::format_tokens(context.tokens)
                } else {
                    "unknown".into()
                },
                context.est_spend_usd
            ),
            evidence: "why this showed: local aggregates for selected period".into(),
        });
    }

    let _ = estimate_spend_usd;
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
