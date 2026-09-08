//! Localhost HTTP server with embedded HTML.
use crate::insights::build_insights;
use crate::models::AdapterStatus;
use crate::rollups::rollup;
use crate::SessionStore;
use anyhow::Result;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tiny_http::{Header, Method, Response, Server, StatusCode};

const HTML: &str = include_str!("../static/app.html");

#[derive(Serialize)]
struct DashboardJson {
    today: crate::models::PeriodRollup,
    insights: crate::models::InsightsPayload,
    used_fixtures: bool,
}

pub fn serve(addr: &str, store: Arc<Mutex<SessionStore>>) -> Result<()> {
    let server = Server::http(addr).map_err(|e| anyhow::anyhow!("bind {addr}: {e}"))?;
    eprintln!("argus-local listening on http://{addr}");
    for request in server.incoming_requests() {
        let url_raw = request.url().to_string();
        let path = url_raw.split('?').next().unwrap_or("/").to_string();
        let method = request.method().clone();
        let response = match (method, path.as_str()) {
            (Method::Get, "/") | (Method::Get, "/index.html") | (Method::Get, "/today") | (Method::Get, "/insights") => {
                Response::from_string(HTML)
                    .with_header(Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap())
            }
            (Method::Get, path) if path.starts_with("/api/dashboard") => {
                let days = parse_days(&url_raw);
                let payload = {
                    let st = store.lock().unwrap();
                    let statuses = st.statuses.clone();
                    let used_fixtures = st.used_fixtures;
                    let today = rollup(&st.sessions, days, statuses.clone(), used_fixtures);
                    let insights = build_insights(&st.sessions, days, statuses, used_fixtures);
                    DashboardJson {
                        today,
                        insights,
                        used_fixtures: st.used_fixtures,
                    }
                };
                let body = serde_json::to_string(&payload)?;
                Response::from_string(body)
                    .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap())
            }
            (Method::Get, "/api/health") => Response::from_string(r#"{"ok":true}"#)
                .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap()),
            _ => Response::from_string("not found").with_status_code(StatusCode(404)),
        };
        let _ = request.respond(response);
    }
    Ok(())
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

pub fn open_browser(url: &str) {
    if let Err(e) = open::that(url) {
        eprintln!("argus-local: could not open browser: {e}");
    }
}

// silence unused in some builds
#[allow(dead_code)]
fn _status_ok() -> AdapterStatus {
    AdapterStatus {
        name: "ok".into(),
        ok: true,
        partial: false,
        detail: String::new(),
    }
}
