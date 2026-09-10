//! Deterministic insights - observations with evidence, never grades.
use crate::adapters::{load_session_coaching, load_session_telemetry};
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
    let since = Utc::now() - Duration::days(period_days as i64);

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
                .filter(|s| s.started_at >= since)
                .map(|s| s.tools_accepted)
                .sum();
            let proposed: i64 = sessions
                .iter()
                .filter(|s| s.started_at >= since)
                .map(|s| s.tools_proposed)
                .sum();
            findings.push(Finding {
                title: "Tool accept stayed high".into(),
                summary: format!("{pct:.0}% accept \u{2014} {} sessions", context.sessions),
                evidence: format!("why this showed: {accepted}/{proposed} proposed tools accepted"),
            });
        }
    }

    // 3) Incomplete cost / missing I/O — suppress when turn usage exists on disk (ARG-38).
    // Grok scan rows may still mark cost_complete=false (context tokens in rollups), but
    // updates.jsonl turn_completed.usage can provide billable I/O on session detail.
    let grok_has_turn_io = period_has_grok_turn_io(sessions, since);
    for t in &context.by_tool {
        let is_grok = t.tool == "Grok";
        if !t.cost_incomplete && !is_grok {
            continue;
        }
        if is_grok && grok_has_turn_io {
            // Align with detail: I/O exists; don't claim "no I/O".
            // Optional soft note distinguishing context rollups vs detail I/O — skip noisy finding.
            continue;
        }
        if t.cost_incomplete || (is_grok && !grok_has_turn_io) {
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
        let grok_partial = context
            .adapter_status
            .iter()
            .any(|a| a.partial && a.name.contains("Grok"));
        if grok_partial && !grok_has_turn_io {
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

    // 6) ARG-40 coaching observations (Grok-first) — titles are coaching tags, not a score.
    push_coaching_findings(&mut findings, sessions, since);

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

/// True when any Grok session in the period has turn_completed usage I/O on disk.
fn period_has_grok_turn_io(sessions: &[SessionRecord], since: chrono::DateTime<Utc>) -> bool {
    sessions
        .iter()
        .filter(|s| s.source == "grok" && s.started_at >= since)
        .any(|s| match load_session_telemetry(&s.id) {
            Ok(tel) => {
                tel.input_tokens.unwrap_or(0) > 0 || tel.output_tokens.unwrap_or(0) > 0
            }
            Err(_) => false,
        })
}

fn push_coaching_findings(
    findings: &mut Vec<Finding>,
    sessions: &[SessionRecord],
    since: chrono::DateTime<Utc>,
) {
    let mut best_streak: Option<(String, i64, String)> = None; // name, streak, session id
    let mut fail_sessions = 0i64;
    let mut fail_total = 0i64;
    let mut fail_calls = 0i64;
    let mut explore = 0i64;
    let mut act = 0i64;
    let mut checks = 0i64;
    let mut edits = 0i64;
    let mut subagent_spawns = 0i64;
    let mut subagent_dirs = 0i64;
    let mut coaching_sessions = 0i64;

    for s in sessions.iter().filter(|s| s.source == "grok" && s.started_at >= since) {
        let Ok(obs) = load_session_coaching(&s.id) else {
            continue;
        };
        let mut any = false;
        if let Some(n) = obs.tool_failure_count {
            if n > 0 {
                fail_sessions += 1;
                fail_total += n;
                fail_calls += obs.tool_call_count.unwrap_or(0);
                any = true;
            }
        }
        if let (Some(name), Some(streak)) = (&obs.identical_tool_name, obs.identical_tool_streak) {
            any = true;
            let better = match &best_streak {
                None => true,
                Some((_, prev, _)) => streak > *prev,
            };
            if better {
                best_streak = Some((name.clone(), streak, s.id.clone()));
            }
        }
        if let (Some(e), Some(a)) = (obs.explore_count, obs.act_count) {
            explore += e;
            act += a;
            any = true;
        }
        if let (Some(c), Some(ed)) = (obs.checks_after_edits, obs.edits_count) {
            checks += c;
            edits += ed;
            any = true;
        }
        if let Some(n) = obs.subagent_spawn_count {
            subagent_spawns += n;
            any = true;
        }
        if let Some(n) = obs.subagent_dir_count {
            subagent_dirs += n;
            any = true;
        }
        if any {
            coaching_sessions += 1;
        }
    }

    if coaching_sessions == 0 {
        return;
    }

    // Prefer actionable coaching titles (max a few; truncate later handles cap).
    if let Some((name, streak, sid)) = best_streak {
        if streak >= 8 {
            findings.insert(
                0,
                Finding {
                    title: "Long identical tool run".into(),
                    summary: format!("`{name}` ran {streak} times in a row"),
                    evidence: format!(
                        "why this showed: consecutive tool_started events in {sid}"
                    ),
                },
            );
        } else if streak >= 3 {
            findings.insert(
                0,
                Finding {
                    title: "Identical tool run".into(),
                    summary: format!("`{name}` ran {streak} times in a row"),
                    evidence: format!(
                        "why this showed: consecutive tool_started events in {sid}"
                    ),
                },
            );
        }
    }

    if fail_total > 0 {
        let denom = if fail_calls > 0 {
            format!(" out of {fail_calls} tool calls")
        } else {
            String::new()
        };
        findings.insert(
            0,
            Finding {
                title: "Tool failures observed".into(),
                summary: format!(
                    "{fail_total} tool failure(s){denom} across {fail_sessions} session(s)"
                ),
                evidence: "why this showed: signals.toolFailureCount / events tool_completed.outcome=error"
                    .into(),
            },
        );
    }

    if edits > 0 {
        let rate = (checks as f64) * 100.0 / (edits as f64);
        let title = if rate < 50.0 {
            "Few checks after edits"
        } else {
            "Checks after edits"
        };
        findings.push(Finding {
            title: title.into(),
            summary: format!("{checks} of {edits} write-like calls followed by read/grep/terminal within 5 steps"),
            evidence: "why this showed: tool_started sequences (write/replace then explore/terminal)".into(),
        });
    }

    if explore + act > 0 {
        findings.push(Finding {
            title: "Explore vs act mix".into(),
            summary: format!("{explore} explore-ish vs {act} act-ish tool starts"),
            evidence: "why this showed: classified tool_started names (read/grep/list vs write/replace/terminal)".into(),
        });
    }

    if subagent_spawns > 0 || subagent_dirs > 0 {
        let mut parts = Vec::new();
        if subagent_spawns > 0 {
            parts.push(format!("{subagent_spawns} spawn_subagent"));
        }
        if subagent_dirs > 0 {
            parts.push(format!("{subagent_dirs} subagent dir(s)"));
        }
        findings.push(Finding {
            title: "Subagents used".into(),
            summary: parts.join(" · "),
            evidence: "why this showed: spawn_subagent tool_started and/or session/subagents/ dirs".into(),
        });
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
