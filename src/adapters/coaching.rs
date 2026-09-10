//! Grok-first v1 coaching observations (ARG-40) + coach dimensions (ARG-42).
//! Deterministic heuristics from signals.json + events.jsonl.
//! Never reads auth.json, prompt_context.json, system_prompt, rawOutput, or prompt bodies.
//! Observations / coach labels only - not a score. Hide missing.
use super::grok::grok_roots;
use crate::models::{CoachDimension, CoachLabel, SessionCoachingObs};
use anyhow::Result;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const CHECKS_WINDOW: usize = 5;
const MIN_IDENTICAL_STREAK: i64 = 3;

/// Load coaching observations for a session id like `grok:{uuid}`.
/// Claude/Cursor: empty (hide-missing stubs until parity).
pub fn load_session_coaching(id: &str) -> Result<SessionCoachingObs> {
    let (tool, raw_id) = split_id(id);
    let mut obs = SessionCoachingObs::default();
    if tool != "grok" {
        return Ok(obs);
    }
    let Some(dir) = find_grok_session_dir(raw_id) else {
        return Ok(obs);
    };
    fill_from_session_dir(&mut obs, &dir);
    Ok(obs)
}

fn split_id(id: &str) -> (&str, &str) {
    if let Some((prefix, rest)) = id.split_once(':') {
        (prefix, rest)
    } else {
        ("unknown", id)
    }
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
                if path.join("signals.json").exists() || path.join("events.jsonl").exists() {
                    return Some(path.to_path_buf());
                }
            }
        }
    }
    None
}

fn fill_from_session_dir(obs: &mut SessionCoachingObs, dir: &Path) {
    // - signals.json numeric fields (failures) -
    let signals_path = dir.join("signals.json");
    if signals_path.exists() {
        if let Ok(raw) = fs::read_to_string(&signals_path) {
            if let Ok(v) = serde_json::from_str::<Value>(&raw) {
                let fail = i64_field(&v, "toolFailureCount");
                let err = i64_field(&v, "errorCount");
                // Prefer toolFailureCount; fall back to errorCount when failure absent.
                let chosen = fail.or(err);
                if let Some(n) = chosen {
                    if n > 0 {
                        obs.tool_failure_count = Some(n);
                    }
                }
                if let Some(tc) = i64_field(&v, "toolCallCount") {
                    obs.tool_call_count = Some(tc);
                }
            }
        }
    }

    // - events.jsonl: starts + completed outcomes (names/outcomes only) -
    let events_path = dir.join("events.jsonl");
    let mut starts: Vec<String> = Vec::new();
    let mut event_errors: i64 = 0;
    let mut event_completed: i64 = 0;
    if events_path.exists() {
        if let Ok(raw) = fs::read_to_string(&events_path) {
            for line in raw.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let Ok(v) = serde_json::from_str::<Value>(line) else {
                    continue;
                };
                let kind = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
                match kind {
                    "tool_started" => {
                        if let Some(name) = v.get("tool_name").and_then(|x| x.as_str()) {
                            if !name.is_empty() {
                                starts.push(name.to_string());
                            }
                        }
                    }
                    "tool_completed" => {
                        event_completed += 1;
                        let outcome = v.get("outcome").and_then(|x| x.as_str()).unwrap_or("");
                        if outcome == "error" {
                            event_errors += 1;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // If signals lacked failures but events show errors, surface those.
    if obs.tool_failure_count.is_none() && event_errors > 0 {
        obs.tool_failure_count = Some(event_errors);
    }
    if event_errors > 0 {
        obs.tool_error_from_events = Some(event_errors);
    }
    if event_completed > 0 && obs.tool_call_count.is_none() {
        obs.tool_call_count = Some(event_completed);
    }

    // Identical consecutive tool runs
    if let Some((name, streak)) = max_identical_streak(&starts) {
        if streak >= MIN_IDENTICAL_STREAK {
            obs.identical_tool_name = Some(name);
            obs.identical_tool_streak = Some(streak);
        }
    }

    // Explore vs act
    let mut explore: i64 = 0;
    let mut act: i64 = 0;
    let mut other: i64 = 0;
    for name in &starts {
        match classify_tool(name) {
            ToolKind::Explore => explore += 1,
            ToolKind::Act => act += 1,
            ToolKind::Other => other += 1,
        }
    }
    if explore + act + other > 0 {
        obs.explore_count = Some(explore);
        obs.act_count = Some(act);
        if other > 0 {
            obs.other_tool_count = Some(other);
        }
    }

    // Checks after edits (write/replace followed by explore or terminal within N)
    let mut edits: i64 = 0;
    let mut checked: i64 = 0;
    for (i, name) in starts.iter().enumerate() {
        if is_write_like(name) {
            edits += 1;
            let end = (i + 1 + CHECKS_WINDOW).min(starts.len());
            let window = &starts[i + 1..end];
            if window.iter().any(|n| is_check_like(n)) {
                checked += 1;
            }
        }
    }
    if edits > 0 {
        obs.checks_after_edits = Some(checked);
        obs.edits_count = Some(edits);
        obs.checks_window = Some(CHECKS_WINDOW as i64);
    }

    // Subagents: spawn_subagent tool starts + subagents/ dir count
    let spawn = starts.iter().filter(|n| n.as_str() == "spawn_subagent").count() as i64;
    if spawn > 0 {
        obs.subagent_spawn_count = Some(spawn);
    }
    let sub_dir = dir.join("subagents");
    if sub_dir.is_dir() {
        let n = fs::read_dir(&sub_dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| e.path().is_dir())
                    .count() as i64
            })
            .unwrap_or(0);
        if n > 0 {
            obs.subagent_dir_count = Some(n);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ToolKind {
    Explore,
    Act,
    Other,
}

fn classify_tool(name: &str) -> ToolKind {
    let n = name.to_ascii_lowercase();
    // Act first so search_replace / write_* are not swallowed by search_* explore rules.
    if is_act(&n) {
        ToolKind::Act
    } else if is_explore(&n) {
        ToolKind::Explore
    } else {
        ToolKind::Other
    }
}

fn is_explore(n: &str) -> bool {
    matches!(
        n,
        "read_file"
            | "read"
            | "grep"
            | "list_dir"
            | "list"
            | "search_tool"
            | "web_search"
            | "web_fetch"
            | "glob"
            | "x_search"
    ) || n.starts_with("read")
        || n.starts_with("list")
        || n.contains("grep")
        || n.starts_with("search")
        || n.contains("glob")
}

fn is_act(n: &str) -> bool {
    matches!(
        n,
        "write"
            | "search_replace"
            | "run_terminal_command"
            | "todo_write"
            | "str_replace"
            | "edit"
            | "bash"
            | "terminal"
    ) || n.contains("write")
        || n.contains("replace")
        || n.contains("terminal")
        || n.contains("edit")
}

fn is_write_like(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    matches!(n.as_str(), "write" | "search_replace" | "str_replace" | "edit")
        || ((n.contains("write") || n.contains("replace") || n.contains("edit"))
            && !n.contains("todo")
            && !n.starts_with("read"))
}

fn is_check_like(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    is_explore(&n) || n.contains("terminal") || n == "run_terminal_command" || n.contains("bash")
}

fn max_identical_streak(starts: &[String]) -> Option<(String, i64)> {
    if starts.is_empty() {
        return None;
    }
    let mut best_name = starts[0].clone();
    let mut best: i64 = 1;
    let mut cur: i64 = 1;
    for i in 1..starts.len() {
        if starts[i] == starts[i - 1] {
            cur += 1;
            if cur > best {
                best = cur;
                best_name = starts[i].clone();
            }
        } else {
            cur = 1;
        }
    }
    Some((best_name, best))
}

fn i64_field(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(|x| {
        x.as_i64()
            .or_else(|| x.as_u64().map(|u| u as i64))
            .or_else(|| x.as_f64().map(|f| f as i64))
    })
}

/// True when any coaching field is present (for UI section visibility).
pub fn coaching_has_any(obs: &SessionCoachingObs) -> bool {
    obs.tool_failure_count.is_some()
        || obs.identical_tool_streak.is_some()
        || obs.explore_count.is_some()
        || obs.checks_after_edits.is_some()
        || obs.subagent_spawn_count.is_some()
        || obs.subagent_dir_count.is_some()
}

/// ARG-42: deterministic coach dimensions from ARG-40 observations.
/// Coach, don't rank - labels + tip only. Hide missing. No composite score.
pub fn coach_dimensions(obs: &SessionCoachingObs) -> Vec<CoachDimension> {
    let mut out = Vec::with_capacity(4);

    // 1. Verification habit - hide if edits=0 / absent
    if let (Some(checks), Some(edits)) = (obs.checks_after_edits, obs.edits_count) {
        if edits > 0 {
            let ratio = checks as f64 / edits as f64;
            let (label, tip) = if ratio >= 0.7 {
                (
                    CoachLabel::Observed,
                    format!(
                        "Checks followed {checks} of {edits} edits - keep verifying after writes before the next change."
                    ),
                )
            } else if ratio >= 0.3 {
                (
                    CoachLabel::Watch,
                    format!(
                        "Checks after only {checks} of {edits} edits - after a write, run a quick read or test before the next change."
                    ),
                )
            } else {
                (
                    CoachLabel::Thin,
                    format!(
                        "Few checks after edits ({checks} of {edits}) - after a write, run a quick read or test before the next change."
                    ),
                )
            };
            out.push(CoachDimension {
                id: "verification_habit".into(),
                title: "Verification habit".into(),
                label,
                tip,
            });
        }
    }

    // 2. Tool thrash / loops - watch if streak-8 or fail rate-5%; observed if streak-3; hide if both zero
    {
        let streak = obs.identical_tool_streak.unwrap_or(0);
        let fails = obs.tool_failure_count.unwrap_or(0);
        let calls = obs.tool_call_count.unwrap_or(0);
        let fail_rate = if calls > 0 {
            fails as f64 / calls as f64
        } else {
            0.0
        };
        let both_zero = streak == 0 && fails == 0;
        if !both_zero {
            if streak >= 8 || fail_rate >= 0.05 {
                let tip = if streak >= 8 {
                    let name = obs
                        .identical_tool_name
                        .as_deref()
                        .unwrap_or("tool");
                    format!(
                        "`{name}` repeated x{streak} - tighten the ask or change approach instead of retrying the same call."
                    )
                } else {
                    format!(
                        "Tool failures are high ({fails} / {calls} calls) - pause, inspect the error, and change approach before another retry."
                    )
                };
                out.push(CoachDimension {
                    id: "tool_thrash".into(),
                    title: "Tool thrash / loops".into(),
                    label: CoachLabel::Watch,
                    tip,
                });
            } else if streak >= 3 {
                let name = obs
                    .identical_tool_name
                    .as_deref()
                    .unwrap_or("tool");
                out.push(CoachDimension {
                    id: "tool_thrash".into(),
                    title: "Tool thrash / loops".into(),
                    label: CoachLabel::Observed,
                    tip: format!(
                        "Identical `{name}` streak x{streak} noted - watch for loops; vary the approach if it repeats."
                    ),
                });
            }
            // else: failures present but rate <5% and streak <3 - hide (not informative)
        }
    }

    // 3. Explore/act balance - hide if both 0 / absent
    {
        let explore = obs.explore_count.unwrap_or(0);
        let act = obs.act_count.unwrap_or(0);
        let total = explore + act;
        if total > 0 {
            let act_share = act as f64 / total as f64;
            let explore_share = explore as f64 / total as f64;
            let (label, tip) = if act_share >= 0.75 {
                (
                    CoachLabel::Watch,
                    format!(
                        "Act-heavy ({act} act / {explore} explore) - skim the file or search before another edit."
                    ),
                )
            } else if explore_share >= 0.75 {
                (
                    CoachLabel::Watch,
                    format!(
                        "Explore-heavy ({explore} explore / {act} act) - after enough context, make a small decisive change."
                    ),
                )
            } else {
                (
                    CoachLabel::Observed,
                    format!(
                        "Mixed explore/act ({explore} explore / {act} act) - keep alternating look-then-change."
                    ),
                )
            };
            out.push(CoachDimension {
                id: "explore_act_balance".into(),
                title: "Explore/act balance".into(),
                label,
                tip,
            });
        }
    }

    // 4. Delegation - spawns-1 observed; dirs without spawns watch; hide if absent
    match (obs.subagent_spawn_count, obs.subagent_dir_count) {
        (Some(spawns), _) if spawns >= 1 => {
            out.push(CoachDimension {
                id: "delegation".into(),
                title: "Delegation".into(),
                label: CoachLabel::Observed,
                tip: format!(
                    "Subagents were used ({spawns} spawned) - keep scope narrow so the parent stays the source of truth."
                ),
            });
        }
        (None, Some(dirs)) if dirs > 0 => {
            out.push(CoachDimension {
                id: "delegation".into(),
                title: "Delegation".into(),
                label: CoachLabel::Watch,
                tip: format!(
                    "{dirs} subagent dir(s) without spawn_subagent starts - confirm delegation is intentional and scoped."
                ),
            });
        }
        _ => {}
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_thresholds() {
        let mut obs = SessionCoachingObs::default();
        obs.checks_after_edits = Some(8);
        obs.edits_count = Some(10);
        let d = coach_dimensions(&obs);
        assert_eq!(d[0].id, "verification_habit");
        assert_eq!(d[0].label, CoachLabel::Observed);

        obs.checks_after_edits = Some(4);
        let d = coach_dimensions(&obs);
        assert_eq!(d[0].label, CoachLabel::Watch);

        obs.checks_after_edits = Some(1);
        let d = coach_dimensions(&obs);
        assert_eq!(d[0].label, CoachLabel::Thin);

        obs.edits_count = Some(0);
        assert!(coach_dimensions(&obs).is_empty());
    }

    #[test]
    fn thrash_and_balance_and_delegation() {
        let mut obs = SessionCoachingObs::default();
        obs.identical_tool_name = Some("search_replace".into());
        obs.identical_tool_streak = Some(22);
        obs.tool_failure_count = Some(15);
        obs.tool_call_count = Some(671);
        obs.explore_count = Some(197);
        obs.act_count = Some(346);
        obs.subagent_spawn_count = Some(2);
        let d = coach_dimensions(&obs);
        assert!(d.iter().any(|x| x.id == "tool_thrash" && x.label == CoachLabel::Watch));
        assert!(d.iter().any(|x| x.id == "explore_act_balance" && x.label == CoachLabel::Observed));
        assert!(d.iter().any(|x| x.id == "delegation" && x.label == CoachLabel::Observed));
    }
}
