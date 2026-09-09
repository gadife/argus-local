//! Opt-in GitHub shipping adapter — merged PRs / commits / files (local scan).
//! Auth: ARGUS_GITHUB_TOKEN and/or authenticated `gh` CLI. Never invents counts when unconfigured.
use crate::models::{AdapterStatus, ShippingEvent};
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde_json::Value;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct GitHubScan {
    pub events: Vec<ShippingEvent>,
    pub status: AdapterStatus,
    /// True when user opted in and auth resolved (even if zero events).
    pub configured: bool,
    pub auth_source: Option<String>,
    pub login: Option<String>,
}

pub fn settings_path_display() -> String {
    crate::settings::settings_path_display()
}

/// Opt-in: ARGUS_GITHUB_TOKEN, ARGUS_GITHUB_ENABLED, or settings.json github.enabled.
pub fn is_opted_in() -> bool {
    crate::settings::github_opted_in()
}

fn resolve_token() -> Option<(String, String)> {
    if let Ok(t) = std::env::var("ARGUS_GITHUB_TOKEN") {
        let t = t.trim().to_string();
        if !t.is_empty() {
            return Some((t, "env".into()));
        }
    }
    // Prefer gh CLI when available / authenticated.
    if which_gh() {
        if let Ok(out) = Command::new("gh").args(["auth", "token"]).output() {
            if out.status.success() {
                let t = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !t.is_empty() {
                    return Some((t, "gh".into()));
                }
            }
        }
    }
    None
}

fn which_gh() -> bool {
    Command::new("gh")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn gh_api_json(token: &str, path_and_query: &str, accept: Option<&str>) -> Result<Value> {
    // Prefer gh CLI (handles auth); fall back to ureq with PAT.
    if which_gh() {
        let mut cmd = Command::new("gh");
        cmd.args(["api", path_and_query]);
        if let Some(a) = accept {
            cmd.args(["-H", &format!("Accept: {a}")]);
        }
        // Pass token without printing it; gh also uses keyring when already logged in.
        cmd.env("GH_TOKEN", token);
        cmd.env("GITHUB_TOKEN", token);
        let out = cmd.output().context("spawn gh api")?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(anyhow!("gh api failed: {err}"));
        }
        let body = String::from_utf8_lossy(&out.stdout);
        return serde_json::from_str(&body).context("parse gh api json");
    }

    let url = format!(
        "https://api.github.com/{}",
        path_and_query.trim_start_matches('/')
    );
    let mut req = ureq::get(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("User-Agent", "argus-local")
        .set("X-GitHub-Api-Version", "2022-11-28");
    if let Some(a) = accept {
        req = req.set("Accept", a);
    } else {
        req = req.set("Accept", "application/vnd.github+json");
    }
    let resp = req.call().map_err(|e| anyhow!("github http: {e}"))?;
    let body = resp.into_string().context("read github body")?;
    serde_json::from_str(&body).context("parse github json")
}

fn absent_scan(detail: &str) -> GitHubScan {
    GitHubScan {
        events: vec![],
        status: AdapterStatus {
            name: "GitHub".into(),
            ok: false,
            partial: true,
            detail: detail.into(),
        },
        configured: false,
        auth_source: None,
        login: None,
    }
}

/// Scan last ~90d of merged PRs + commits for the authenticated user when opted in.
pub fn scan_github() -> Result<GitHubScan> {
    if !is_opted_in() {
        return Ok(absent_scan(&format!(
            "not configured — enable github.enabled in {} or set ARGUS_GITHUB_TOKEN / ARGUS_GITHUB_ENABLED",
            settings_path_display()
        )));
    }

    let Some((token, auth_source)) = resolve_token() else {
        return Ok(GitHubScan {
            events: vec![],
            status: AdapterStatus {
                name: "GitHub".into(),
                ok: false,
                partial: true,
                detail: "opted in but no PAT / gh auth — set ARGUS_GITHUB_TOKEN or run gh auth login"
                    .into(),
            },
            configured: true,
            auth_source: None,
            login: None,
        });
    };

    let user = gh_api_json(&token, "user", None).context("github user")?;
    let login = user
        .get("login")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("github user missing login"))?
        .to_string();

    let since = (Utc::now() - Duration::days(90))
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();

    let mut events = Vec::new();
    match fetch_merged_prs(&token, &login, &since) {
        Ok(mut prs) => events.append(&mut prs),
        Err(e) => eprintln!("argus-local: github PR scan error: {e}"),
    }
    match fetch_commits(&token, &login, &since) {
        Ok(mut commits) => events.append(&mut commits),
        Err(e) => eprintln!("argus-local: github commit scan error: {e}"),
    }

    let pr_n = events.iter().filter(|e| e.kind == "pr").count();
    let commit_n = events.iter().filter(|e| e.kind == "commit").count();
    Ok(GitHubScan {
        status: AdapterStatus {
            name: "GitHub".into(),
            ok: true,
            partial: false,
            detail: format!(
                "{login} via {auth_source}: {pr_n} merged PRs, {commit_n} commits (90d cache)"
            ),
        },
        events,
        configured: true,
        auth_source: Some(auth_source),
        login: Some(login),
    })
}

fn fetch_merged_prs(token: &str, login: &str, since: &str) -> Result<Vec<ShippingEvent>> {
    let q = format!("author:{login} is:pr is:merged merged:>={since}");
    let encoded = urlencoding_encode(&q);
    let mut page = 1u32;
    let mut out = Vec::new();
    while page <= 3 {
        let path = format!(
            "search/issues?q={encoded}&per_page=50&page={page}&sort=updated&order=desc"
        );
        let v = gh_api_json(token, &path, None)?;
        let items = v
            .get("items")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default();
        if items.is_empty() {
            break;
        }
        for item in items {
            let number = item.get("number").and_then(|x| x.as_i64()).unwrap_or(0);
            let title = item
                .get("title")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let closed = item
                .get("closed_at")
                .and_then(|x| x.as_str())
                .or_else(|| item.get("updated_at").and_then(|x| x.as_str()))
                .unwrap_or("");
            let occurred = parse_rfc3339(closed).unwrap_or_else(Utc::now);
            let repo_url = item
                .get("repository_url")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let repo = repo_url
                .trim_start_matches("https://api.github.com/repos/")
                .to_string();
            let files = fetch_pr_changed_files(token, &repo, number).unwrap_or(0);
            out.push(ShippingEvent {
                id: format!("pr:{repo}:{number}"),
                kind: "pr".into(),
                occurred_at: occurred,
                files_touched: files,
                repo: repo.clone(),
                title,
            });
        }
        let total = v.get("total_count").and_then(|x| x.as_u64()).unwrap_or(0);
        if (page as u64) * 50 >= total {
            break;
        }
        page += 1;
    }
    Ok(out)
}

fn fetch_pr_changed_files(token: &str, repo: &str, number: i64) -> Result<i64> {
    if repo.is_empty() || number <= 0 {
        return Ok(0);
    }
    let path = format!("repos/{repo}/pulls/{number}");
    let v = gh_api_json(token, &path, None)?;
    Ok(v
        .get("changed_files")
        .and_then(|x| x.as_i64())
        .unwrap_or(0))
}

fn fetch_commits(token: &str, login: &str, since: &str) -> Result<Vec<ShippingEvent>> {
    let q = format!("author:{login} committer-date:>={since}");
    let encoded = urlencoding_encode(&q);
    let mut page = 1u32;
    let mut out = Vec::new();
    let accept = "application/vnd.github+json";
    while page <= 3 {
        let path = format!("search/commits?q={encoded}&per_page=50&page={page}");
        let v = gh_api_json(token, &path, Some(accept))?;
        let items = v
            .get("items")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default();
        if items.is_empty() {
            break;
        }
        for item in items {
            let sha = item
                .get("sha")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let date = item
                .pointer("/commit/committer/date")
                .and_then(|x| x.as_str())
                .or_else(|| item.pointer("/commit/author/date").and_then(|x| x.as_str()))
                .unwrap_or("");
            let occurred = parse_rfc3339(date).unwrap_or_else(Utc::now);
            let repo = item
                .pointer("/repository/full_name")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let title = item
                .pointer("/commit/message")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .lines()
                .next()
                .unwrap_or("")
                .to_string();
            // Commit search does not include file counts cheaply; PRs carry files_touched.
            out.push(ShippingEvent {
                id: format!("commit:{sha}"),
                kind: "commit".into(),
                occurred_at: occurred,
                files_touched: 0,
                repo,
                title,
            });
        }
        let total = v.get("total_count").and_then(|x| x.as_u64()).unwrap_or(0);
        if (page as u64) * 50 >= total {
            break;
        }
        page += 1;
    }
    Ok(out)
}

fn parse_rfc3339(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc))
        })
}

fn urlencoding_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Roll events into period counts. Unconfigured → stub (no invented numbers).
pub fn snapshot_for_period(
    configured: bool,
    events: &[ShippingEvent],
    period_days: u32,
    auth_source: Option<&str>,
    login: Option<&str>,
) -> crate::models::ShippingStub {
    if !configured {
        return crate::models::ShippingStub {
            is_stub: true,
            configured: false,
            merged_prs: None,
            commits: None,
            files_touched: None,
            note: format!(
                "GitHub shipping not configured — enable github.enabled in {} or set ARGUS_GITHUB_TOKEN / ARGUS_GITHUB_ENABLED (uses gh when authenticated). No observed PR/commit counts.",
                settings_path_display()
            ),
            auth_source: None,
            login: None,
        };
    }
    let since = Utc::now() - Duration::days(period_days as i64);
    let prs: Vec<&ShippingEvent> = events
        .iter()
        .filter(|e| e.kind == "pr" && e.occurred_at >= since)
        .collect();
    let commits: Vec<&ShippingEvent> = events
        .iter()
        .filter(|e| e.kind == "commit" && e.occurred_at >= since)
        .collect();
    let files: i64 = prs.iter().map(|e| e.files_touched).sum();
    let who = login.unwrap_or("github");
    let via = auth_source.unwrap_or("token");
    crate::models::ShippingStub {
        is_stub: false,
        configured: true,
        merged_prs: Some(prs.len() as i64),
        commits: Some(commits.len() as i64),
        files_touched: Some(files),
        note: format!(
            "{who} via {via} · last {period_days}d — correlation with sessions — not a productivity score. Files touched = sum of changed_files on merged PRs in period."
        ),
        auth_source: auth_source.map(|s| s.to_string()),
        login: login.map(|s| s.to_string()),
    }
}
