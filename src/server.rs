//! Localhost HTTP server with embedded HTML.
use crate::adapters::{coaching_has_any, load_session_coaching, load_session_texts, load_session_telemetry, SessionTexts};
use crate::insights::build_insights;
use crate::models::{AdapterStatus, LanguageLock, SessionDetail, SessionTelemetry};
use crate::rollups::{activity_row_for, rollup, ShippingContext};
use crate::settings;
use crate::SessionStore;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::sync::{Arc, Mutex};
use tiny_http::{Header, Method, Response, Server, StatusCode};

const HTML: &str = include_str!("../static/app.html");

#[derive(Serialize)]
struct DashboardJson {
    today: crate::models::PeriodRollup,
    insights: crate::models::InsightsPayload,
    used_fixtures: bool,
    github_configured: bool,
    prompts_enabled: bool,
}

#[derive(Serialize)]
struct SettingsJson {
    prompts_enabled: bool,
    github_configured: bool,
    github_opted_in: bool,
    settings_path: String,
    share_enabled: bool,
}

#[derive(Deserialize)]
struct PromptsBody {
    enabled: bool,
}

pub fn serve(addr: &str, store: Arc<Mutex<SessionStore>>) -> Result<()> {
    let server = Server::http(addr).map_err(|e| anyhow::anyhow!("bind {addr}: {e}"))?;
    eprintln!("argus-local listening on http://{addr}");
    for mut request in server.incoming_requests() {
        let url_raw = request.url().to_string();
        let path = url_raw.split('?').next().unwrap_or("/").to_string();
        let method = request.method().clone();
        let response = match (method.clone(), path.as_str()) {
            (Method::Get, "/")
            | (Method::Get, "/index.html")
            | (Method::Get, "/today")
            | (Method::Get, "/activity")
            | (Method::Get, "/insights")
            | (Method::Get, "/shipping")
            | (Method::Get, "/settings") => Response::from_string(HTML).with_header(
                Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap(),
            ),
            (Method::Get, path) if path.starts_with("/api/dashboard") => {
                let days = parse_days(&url_raw);
                let payload = {
                    let st = store.lock().unwrap();
                    let statuses = st.statuses.clone();
                    let used_fixtures = st.used_fixtures;
                    let prompts_enabled = settings::prompts_enabled();
                    let today = rollup(
                        &st.sessions,
                        days,
                        statuses.clone(),
                        used_fixtures,
                        ShippingContext {
                            configured: st.github_configured,
                            events: &st.shipping_events,
                            auth_source: st.github_auth_source.as_deref(),
                            login: st.github_login.as_deref(),
                        },
                    );
                    let insights = build_insights(
                        &st.sessions,
                        days,
                        statuses,
                        used_fixtures,
                        ShippingContext {
                            configured: st.github_configured,
                            events: &st.shipping_events,
                            auth_source: st.github_auth_source.as_deref(),
                            login: st.github_login.as_deref(),
                        },
                    );
                    DashboardJson {
                        today,
                        insights,
                        used_fixtures: st.used_fixtures,
                        github_configured: st.github_configured,
                        prompts_enabled,
                    }
                };
                json_ok(&payload)?
            }
            (Method::Get, "/api/settings") => {
                let st = store.lock().unwrap();
                let payload = SettingsJson {
                    prompts_enabled: settings::prompts_enabled(),
                    github_configured: st.github_configured,
                    github_opted_in: settings::github_opted_in(),
                    settings_path: settings::settings_path_display(),
                    share_enabled: false,
                };
                json_ok(&payload)?
            }
            (Method::Post, "/api/settings/prompts") => {
                let mut body = String::new();
                let _ = request.as_reader().read_to_string(&mut body);
                match serde_json::from_str::<PromptsBody>(&body) {
                    Ok(b) => {
                        match settings::set_prompts_enabled(b.enabled) {
                            Ok(_) => {
                                let payload = serde_json::json!({
                                    "ok": true,
                                    "prompts_enabled": settings::prompts_enabled(),
                                    "share_enabled": false,
                                    "note": if b.enabled {
                                        "Prompts on (local) \u{2014} never uploaded; Share stays Off"
                                    } else {
                                        "aggregates only \u{2014} no raw prompts"
                                    }
                                });
                                json_ok(&payload)?
                            }
                            Err(e) => json_err(500, &format!("save failed: {e}"))?,
                        }
                    }
                    Err(e) => json_err(400, &format!("bad json: {e}"))?,
                }
            }
            (Method::Get, path) if path.starts_with("/api/session/") => {
                let id = percent_decode(path.trim_start_matches("/api/session/"));
                let days = parse_days(&url_raw);
                let detail = {
                    let st = store.lock().unwrap();
                    let prompts_on = settings::prompts_enabled();
                    build_session_detail(&st, &id, days, prompts_on)
                };
                match detail {
                    Some(d) => json_ok(&d)?,
                    None => json_err(404, "session not found")?,
                }
            }
            (Method::Get, "/api/health") => Response::from_string(r#"{"ok":true}"#).with_header(
                Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
            ),
            _ => Response::from_string("not found").with_status_code(StatusCode(404)),
        };
        let _ = request.respond(response);
    }
    Ok(())
}

fn build_session_detail(
    st: &SessionStore,
    id: &str,
    _days: u32,
    prompts_on: bool,
) -> Option<SessionDetail> {
    let session = st.sessions.iter().find(|s| s.id == id)?;
    let activity = activity_row_for(session, &st.statuses);
    let mut telemetry = load_session_telemetry(id).unwrap_or_else(|_| SessionTelemetry {
        id: id.to_string(),
        tool: session.tool.clone(),
        ..Default::default()
    });

    // Claude/Cursor: map billable I/O from SessionRecord when disk usage absent.
    if telemetry.input_tokens.is_none() && session.tokens_known && session.cost_complete {
        telemetry.input_tokens = Some(session.input_tokens);
        telemetry.output_tokens = Some(session.output_tokens);
        telemetry.total_tokens = Some(session.input_tokens + session.output_tokens);
    }
    if telemetry.model.is_none() && !session.model.is_empty() {
        telemetry.model = Some(session.model.clone());
    }
    if telemetry.tool.is_empty() {
        telemetry.tool = session.tool.clone();
    }

    let mut prompt_text = None;
    let mut response_text = None;
    let mut prompts_missing = false;
    if prompts_on {
        let texts: SessionTexts = load_session_texts(&activity.id).unwrap_or_default();
        prompt_text = texts.prompt_text;
        response_text = texts.response_text;
        prompts_missing = prompt_text.is_none() && response_text.is_none();
    }

    let coaching_raw = load_session_coaching(id).unwrap_or_default();
    let coaching = if coaching_has_any(&coaching_raw) {
        Some(coaching_raw)
    } else {
        None
    };

    Some(SessionDetail {
        activity,
        telemetry,
        coaching,
        language_lock: LanguageLock::default(),
        prompts_enabled: prompts_on,
        prompt_text,
        response_text,
        prompts_missing,
    })
}

fn json_ok<T: Serialize>(payload: &T) -> Result<Response<std::io::Cursor<Vec<u8>>>> {
    let body = serde_json::to_string(payload)?;
    Ok(Response::from_string(body).with_header(
        Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
    ))
}

fn json_err(code: u16, msg: &str) -> Result<Response<std::io::Cursor<Vec<u8>>>> {
    let body = serde_json::json!({ "ok": false, "error": msg }).to_string();
    Ok(Response::from_string(body)
        .with_status_code(StatusCode(code))
        .with_header(
            Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
        ))
}

fn parse_days(path: &str) -> u32 {
    if let Some(q) = path.split('?').nth(1) {
        for part in q.split('&') {
            let mut kv = part.split('=');
            if kv.next() == Some("days") {
                if let Some(v) = kv.next() {
                    if let Ok(n) = v.parse::<u32>() {
                        return n.clamp(1, 90);
                    }
                }
            }
        }
    }
    7
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let h = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(h) = h {
                if let Ok(v) = u8::from_str_radix(h, 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

pub fn open_browser(url: &str) {
    if let Err(e) = open::that(url) {
        eprintln!("argus-local: could not open browser: {e}");
    }
}

#[allow(dead_code)]
fn _status_ok() -> AdapterStatus {
    AdapterStatus {
        name: "ok".into(),
        ok: true,
        partial: false,
        detail: String::new(),
    }
}
