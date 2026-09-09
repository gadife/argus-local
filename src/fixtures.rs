//! Demo fixture sessions matching mock KPI shape when no real adapters fire.
use crate::models::{AdapterStatus, SessionRecord};
use chrono::{Duration, Utc};
use uuid::Uuid;

pub fn fixture_sessions() -> Vec<SessionRecord> {
    let now = Utc::now();
    let mut rows = Vec::new();

    // Claude Code - ~23 sessions, ~5.6M tokens
    for i in 0..23 {
        let start = now - Duration::hours(4 + i * 5);
        let dur = if i % 5 == 0 {
            Duration::minutes(75)
        } else {
            Duration::minutes(35 + (i % 20))
        };
        let inp = 180_000 + (i as i64) * 12_000;
        let out = 40_000 + (i as i64) * 3_000;
        rows.push(SessionRecord {
            id: format!("fixture:claude:{}", Uuid::new_v4()),
            tool: "Claude Code".into(),
            model: if i % 4 == 0 {
                "Claude Opus 4.6".into()
            } else {
                "Claude Sonnet 4".into()
            },
            started_at: start,
            ended_at: start + dur,
            input_tokens: inp,
            output_tokens: out,
            tokens_known: true,
            tools_proposed: 8 + (i % 5) as i64,
            tools_accepted: 7 + (i % 4) as i64,
            source: "fixture".into(),
            cost_complete: true,
        });
    }

    // Cursor - ~14 sessions, ~3.8M
    for i in 0..14 {
        let start = now - Duration::hours(3 + i * 7);
        let inp = 200_000 + (i as i64) * 15_000;
        let out = 50_000 + (i as i64) * 4_000;
        rows.push(SessionRecord {
            id: format!("fixture:cursor:{}", Uuid::new_v4()),
            tool: "Cursor".into(),
            model: if i % 3 == 0 {
                "GPT-4.1".into()
            } else {
                "claude-4-sonnet".into()
            },
            started_at: start,
            ended_at: start + Duration::minutes(40 + i),
            input_tokens: inp,
            output_tokens: out,
            tokens_known: true,
            tools_proposed: 6,
            tools_accepted: 5,
            source: "fixture".into(),
            cost_complete: true,
        });
    }

    // Codex - ~8 sessions, ~2.4M
    for i in 0..8 {
        let start = now - Duration::hours(6 + i * 9);
        rows.push(SessionRecord {
            id: format!("fixture:codex:{}", Uuid::new_v4()),
            tool: "Codex".into(),
            model: "Codex Medium".into(),
            started_at: start,
            ended_at: start + Duration::minutes(50),
            input_tokens: 220_000,
            output_tokens: 80_000,
            tokens_known: true,
            tools_proposed: 4,
            tools_accepted: 4,
            source: "fixture".into(),
            cost_complete: true,
        });
    }

    // Grok - ~2 sessions, ~0.6M, cost incomplete
    for i in 0..2 {
        let start = now - Duration::hours(10 + i * 12);
        rows.push(SessionRecord {
            id: format!("fixture:grok:{}", Uuid::new_v4()),
            tool: "Grok".into(),
            model: "Grok".into(),
            started_at: start,
            ended_at: start + Duration::minutes(25),
            input_tokens: 250_000,
            output_tokens: 50_000,
            tokens_known: true,
            tools_proposed: 0,
            tools_accepted: 0,
            source: "fixture".into(),
            cost_complete: false,
        });
    }

    rows
}

pub fn fixture_statuses() -> Vec<AdapterStatus> {
    vec![
        AdapterStatus {
            name: "Claude Code".into(),
            ok: true,
            partial: false,
            detail: "fixture demo data".into(),
        },
        AdapterStatus {
            name: "Cursor".into(),
            ok: true,
            partial: false,
            detail: "fixture demo data".into(),
        },
        AdapterStatus {
            name: "Codex".into(),
            ok: true,
            partial: false,
            detail: "fixture demo data".into(),
        },
        AdapterStatus {
            name: "Grok".into(),
            ok: true,
            partial: true,
            detail: "fixture demo data (cost incomplete)".into(),
        },
    ]
}
