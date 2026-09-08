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
    pub tools_proposed: i64,
    pub tools_accepted: i64,
    pub source: String, // "claude" | "cursor" | "fixture" | "stub"
    pub cost_complete: bool,
}

impl SessionRecord {
    pub fn total_tokens(&self) -> i64 {
        self.input_tokens + self.output_tokens
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
    pub est_spend_usd: f64,
    pub tool_accept_pct: Option<f64>,
    pub models_unique: i64,
    pub by_tool: Vec<ToolRollup>,
    pub activity: Vec<ActivityRow>,
    pub shipping: ShippingStub,
    pub adapter_status: Vec<AdapterStatus>,
    pub language_lock: LanguageLock,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRollup {
    pub tool: String,
    pub tokens: i64,
    pub sessions: i64,
    pub cost_incomplete: bool,
    pub share_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityRow {
    pub time_range: String,
    pub tool: String,
    pub model: String,
    pub tokens: i64,
    pub est_spend_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShippingStub {
    pub merged_prs: i64,
    pub commits: i64,
    pub files_touched: i64,
    pub note: String,
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
            estimates_note: "Local only · estimates ≠ invoice".into(),
            observations_note: "Local only · observations, not a score".into(),
            shipping_note: "correlation with sessions · not a productivity score.".into(),
            footer: "aggregates only · no raw prompts or code · local-first".into(),
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
