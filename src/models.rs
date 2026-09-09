//! Shared domain models for Argus Local.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub tool: String,
    pub model: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    /// False when adapter observed a session but had no real token counts.
    pub tokens_known: bool,
    pub tools_proposed: i64,
    pub tools_accepted: i64,
    pub source: String, // "claude" | "cursor" | "fixture" | "stub"
    pub cost_complete: bool,
}

impl SessionRecord {
    pub fn total_tokens(&self) -> i64 {
        if self.tokens_known {
            self.input_tokens + self.output_tokens
        } else {
            0
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterStatus {
    pub name: String,
    pub ok: bool,
    pub partial: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeriodRollup {
    pub period_days: u32,
    pub sessions: i64,
    pub tokens: i64,
    pub tokens_known: bool,
    pub est_spend_usd: f64,
    pub tool_accept_pct: Option<f64>,
    pub models_unique: i64,
    pub by_tool: Vec<ToolRollup>,
    pub activity: Vec<ActivityRow>,
    pub shipping: ShippingStub,
    pub adapter_status: Vec<AdapterStatus>,
    pub language_lock: LanguageLock,
    pub used_fixtures: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRollup {
    pub tool: String,
    pub tokens: i64,
    pub tokens_known: bool,
    pub sessions: i64,
    pub cost_incomplete: bool,
    pub share_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityRow {
    /// Session id for Activity drill-in (aggregates only — never prompts/code).
    pub id: String,
    pub time_range: String,
    pub started_at: String,
    pub ended_at: String,
    /// Human duration like "12m" / "1h 3m"; empty if unknown.
    pub duration: String,
    pub tool: String,
    pub model: String,
    pub tokens: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub tokens_known: bool,
    pub tools_proposed: i64,
    pub tools_accepted: i64,
    pub cost_complete: bool,
    pub source: String,
    /// Adapter partial/missing note when applicable; empty otherwise.
    pub adapter_note: String,
    pub est_spend_usd: f64,
}

/// Shipping panel payload. When `is_stub` / not configured: hide numbers (no fake counts).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShippingStub {
    /// True when GitHub is unconfigured — UI must not invent PR/commit counts.
    pub is_stub: bool,
    pub configured: bool,
    pub merged_prs: Option<i64>,
    pub commits: Option<i64>,
    pub files_touched: Option<i64>,
    pub note: String,
    pub auth_source: Option<String>,
    pub login: Option<String>,
}

/// Observed GitHub shipping event cached in SQLite (90d window).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShippingEvent {
    pub id: String,
    pub kind: String, // "pr" | "commit"
    pub occurred_at: DateTime<Utc>,
    pub files_touched: i64,
    pub repo: String,
    pub title: String,
}


/// Per-session detail payload (Activity drill-in). Prompt/response only when opted in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDetail {
    pub id: String,
    pub tool: String,
    pub model: String,
    pub started_at: String,
    pub ended_at: String,
    pub duration: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub tokens: i64,
    pub tokens_known: bool,
    pub tools_proposed: i64,
    pub tools_accepted: i64,
    pub cost_complete: bool,
    pub source: String,
    pub adapter_note: String,
    pub est_spend_usd: f64,
    pub prompts_enabled: bool,
    /// Present only when prompts_enabled and adapter file had the field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_text: Option<String>,
    /// True when opt-in is on but this adapter/session has no recoverable text.
    pub prompts_missing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageLock {
    pub estimates_note: String,
    pub observations_note: String,
    pub shipping_note: String,
    pub footer: String,
}

impl Default for LanguageLock {
    fn default() -> Self {
        Self {
            estimates_note: "Local only \u{2014} estimates \u{2260} invoice".into(),
            observations_note: "Local only \u{2014} observations, not a score".into(),
            shipping_note: "correlation with sessions \u{2014} not a productivity score".into(),
            footer: "aggregates only \u{2014} no raw prompts or code \u{2014} local-first".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub title: String,
    pub summary: String,
    pub evidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsightsPayload {
    pub period_days: u32,
    pub findings: Vec<Finding>,
    pub context: PeriodRollup,
    pub language_lock: LanguageLock,
}
