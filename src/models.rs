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

/// Per-session telemetry for Activity detail (ARG-38).
/// All metric fields are Option — omit / hide when absent. Never invent numbers.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionTelemetry {
    pub id: String,
    pub tool: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    // --- Context (distinct from billable I/O) ---
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_tokens_used: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<i64>,
    /// Percent of window used when both context fields present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_pct: Option<f64>,

    // --- Billable I/O (SUM of turn_completed.usage when present) ---
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_read_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_calls: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_duration_ms: Option<i64>,
    /// Estimate from costUsdTicks when present — NOT an invoice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd_estimate: Option<f64>,

    // --- Signals extras ---
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools_used: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_time_to_first_token_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_response_time_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_duration_seconds: Option<i64>,

    // --- Summary ---
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_kind: Option<String>,
    /// Path only (cwd) — never secrets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,

    /// Honesty labels for UI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_note: Option<String>,
}

/// Grok-first coaching observations (ARG-40). All Option — hide when absent.
/// Observations only — never a composite score / peer rank / health %.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionCoachingObs {
    /// From signals.toolFailureCount (or errorCount / events errors when failure absent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_failure_count: Option<i64>,
    /// tool_completed.outcome=error count when events present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_error_from_events: Option<i64>,
    /// Denominator when known (signals.toolCallCount or completed events).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_count: Option<i64>,

    /// Longest consecutive identical tool_started name (when streak >= 3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identical_tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identical_tool_streak: Option<i64>,

    /// Explore-ish vs act-ish tool_started counts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explore_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub act_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_tool_count: Option<i64>,

    /// Write/edit tools followed by read/grep/terminal within checks_window steps.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checks_after_edits: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edits_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checks_window: Option<i64>,

    /// spawn_subagent tool_started count.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subagent_spawn_count: Option<i64>,
    /// Count of directories under session/subagents/.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subagent_dir_count: Option<i64>,
}

/// Coach verdict label (ARG-42). Not a score - observed / watch / thin only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoachLabel {
    Observed,
    Watch,
    Thin,
}

impl CoachLabel {
    pub fn as_str(self) -> &'static str {
        match self {
            CoachLabel::Observed => "observed",
            CoachLabel::Watch => "watch",
            CoachLabel::Thin => "thin",
        }
    }
}

/// One coach dimension: label + tip. No numeric score / peer rank.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoachDimension {
    pub id: String,
    pub title: String,
    pub label: CoachLabel,
    pub tip: String,
}

/// GET /api/session/:id payload - activity + telemetry + coaching + coach dimensions + opt-in prompts (ARG-37/38/40/42).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDetail {
    pub activity: ActivityRow,
    pub telemetry: SessionTelemetry,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coaching: Option<SessionCoachingObs>,
    /// ARG-42 coach dimensions - omit when empty (hide missing).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub coach_dimensions: Vec<CoachDimension>,
    pub language_lock: LanguageLock,
    pub prompts_enabled: bool,
    /// Present only when prompts_enabled and adapter file had the field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_text: Option<String>,
    /// True when opt-in is on but this adapter/session has no recoverable text.
    pub prompts_missing: bool,
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