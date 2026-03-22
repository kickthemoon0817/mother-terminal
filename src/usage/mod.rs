use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Unified usage data for any CLI.
#[derive(Debug, Clone, Default)]
pub struct CLIUsage {
    /// Primary window used percent (0-100). 5h for Claude/Codex, daily for Gemini.
    pub primary_percent: u32,
    /// Secondary window used percent (0-100). Weekly for Claude/Codex, None for Gemini.
    pub secondary_percent: Option<u32>,
    /// Primary window label (e.g., "5h", "24h")
    pub primary_label: String,
    /// Secondary window label (e.g., "wk")
    pub secondary_label: String,
}

impl CLIUsage {
    pub fn format(&self) -> String {
        let p_rem = format_remaining_from_percent(self.primary_percent, &self.primary_label);
        let mut s = format!("{}:{}%({})", self.primary_label, self.primary_percent, p_rem);
        if let Some(sec) = self.secondary_percent {
            let s_rem = format_remaining_from_percent(sec, &self.secondary_label);
            s.push_str(&format!(" {}:{}%({})", self.secondary_label, sec, s_rem));
        }
        s
    }
}

const CACHE_TTL_SECS: u64 = 60;

// ── Claude Usage (Anthropic OAuth API) ───────────────────────────────────

const CLAUDE_OAUTH_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";

pub fn fetch_claude_usage() -> Option<CLIUsage> {
    if let Some(cached) = read_cache("claude") { return Some(cached); }

    let refresh = read_json_field_from_keychain("Claude Code-credentials", &["claudeAiOauth", "refreshToken"])
        .or_else(|| read_json_field_from_file("~/.claude/.credentials.json", &["claudeAiOauth", "refreshToken"]))?;

    let body = format!(r#"{{"grant_type":"refresh_token","refresh_token":"{refresh}","client_id":"{CLAUDE_OAUTH_CLIENT_ID}"}}"#);
    let token = curl_post_json("https://platform.claude.com/v1/oauth/token", &body, None)
        .and_then(|r| r.get("access_token")?.as_str().map(|s| s.to_string()))?;

    let resp = curl_get("https://api.anthropic.com/api/oauth/usage", &token)?;
    if resp.get("error").is_some() { return None; }

    let five = (resp.get("five_hour")?.get("utilization")?.as_f64()? * 100.0).min(100.0) as u32;
    let week = (resp.get("seven_day")?.get("utilization")?.as_f64()? * 100.0).min(100.0) as u32;

    let usage = CLIUsage {
        primary_percent: five, secondary_percent: Some(week),
        primary_label: "5h".into(), secondary_label: "wk".into(),
    };
    write_cache("claude", &usage);
    Some(usage)
}

// ── Codex Usage (OpenAI) ─────────────────────────────────────────────────

pub fn fetch_codex_usage() -> Option<CLIUsage> {
    if let Some(cached) = read_cache("codex") { return Some(cached); }

    let home = dirs::home_dir()?;
    let auth_path = home.join(".codex/auth.json");
    let auth_data = fs::read_to_string(&auth_path).ok()?;
    let auth: serde_json::Value = serde_json::from_str(&auth_data).ok()?;
    let access_token = auth.get("tokens")?.get("access_token")?.as_str()?;

    let resp = curl_get("https://api.openai.com/v1/organization/usage", access_token)
        .or_else(|| curl_get("https://chatgpt.com/backend-api/accounts/check/v4-2023-04-27", access_token));

    if let Some(resp) = resp
        && let Some(rate) = resp.get("rate_limits").or(resp.get("rateLimits")) {
            let primary = rate.get("primary")
                .and_then(|p| p.get("usedPercent"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;
            let secondary = rate.get("secondary")
                .and_then(|p| p.get("usedPercent"))
                .and_then(|v| v.as_u64());

            let usage = CLIUsage {
                primary_percent: primary,
                secondary_percent: secondary.map(|s| s as u32),
                primary_label: "5h".into(),
                secondary_label: "wk".into(),
            };
            write_cache("codex", &usage);
            return Some(usage);
        }

    None
}

// ── Gemini Usage (Google Cloud Code Assist API) ──────────────────────────

pub fn fetch_gemini_usage() -> Option<CLIUsage> {
    if let Some(cached) = read_cache("gemini") { return Some(cached); }

    let home = dirs::home_dir()?;
    let cred_path = home.join(".gemini/oauth_creds.json");
    let cred_data = fs::read_to_string(&cred_path).ok()?;
    let creds: serde_json::Value = serde_json::from_str(&cred_data).ok()?;

    let mut access_token = creds.get("access_token")?.as_str()?.to_string();

    // Check if token is expired and refresh if needed
    if let Some(expiry) = creds.get("expiry_date").and_then(|e| e.as_u64()) {
        let now_ms = now_epoch() * 1000;
        if now_ms > expiry
            && let Some(refresh) = creds.get("refresh_token").and_then(|r| r.as_str()) {
                let body = format!(
                    r#"{{"client_id":"77185425430.apps.googleusercontent.com","client_secret":"OTJgUOQcT7lO7GsGZq2G4IlT","grant_type":"refresh_token","refresh_token":"{refresh}"}}"#
                );
                if let Some(resp) = curl_post_json("https://oauth2.googleapis.com/token", &body, None)
                    && let Some(new_token) = resp.get("access_token").and_then(|t| t.as_str())
                {
                    access_token = new_token.to_string();
                }
            }
    }

    let body = r#"{"project":"_"}"#;
    let resp = curl_post_json(
        "https://cloudcode-pa.googleapis.com/v1beta5:retrieveUserQuota",
        body,
        Some(&access_token),
    )?;

    let buckets = resp.get("buckets")?.as_array()?;
    if buckets.is_empty() { return None; }

    let mut max_used: f64 = 0.0;
    for bucket in buckets {
        let fraction = bucket.get("remainingFraction").and_then(|f| f.as_f64()).unwrap_or(1.0);
        let used = 1.0 - fraction;
        if used > max_used { max_used = used; }
    }

    let usage = CLIUsage {
        primary_percent: (max_used * 100.0).min(100.0) as u32,
        secondary_percent: None,
        primary_label: "24h".into(),
        secondary_label: String::new(),
    };
    write_cache("gemini", &usage);
    Some(usage)
}

// ── Unified format function ──────────────────────────────────────────────

pub fn format_cli_usage(cli: &str) -> String {
    let result = match cli {
        "claude" => fetch_claude_usage(),
        "codex" => fetch_codex_usage(),
        "gemini" => fetch_gemini_usage(),
        _ => None,
    };
    match result {
        Some(u) => u.format(),
        None => "—".to_string(),
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn curl_get(url: &str, token: &str) -> Option<serde_json::Value> {
    let output = std::process::Command::new("curl")
        .args(["-s", "-H", &format!("Authorization: Bearer {token}"), url])
        .output().ok()?;
    if !output.status.success() { return None; }
    serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).ok()
}

fn curl_post_json(url: &str, body: &str, token: Option<&str>) -> Option<serde_json::Value> {
    let mut args = vec!["-s", "-X", "POST", "-H", "Content-Type: application/json", "-d", body];
    let auth_header;
    if let Some(t) = token {
        auth_header = format!("Authorization: Bearer {t}");
        args.extend(["-H", auth_header.as_str()]);
    }
    args.push(url);
    let output = std::process::Command::new("curl").args(&args).output().ok()?;
    if !output.status.success() { return None; }
    serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).ok()
}

fn read_json_field_from_keychain(service: &str, path: &[&str]) -> Option<String> {
    let output = std::process::Command::new("security")
        .args(["find-generic-password", "-s", service, "-w"])
        .output().ok()?;
    if !output.status.success() { return None; }
    let parsed: serde_json::Value = serde_json::from_str(
        String::from_utf8_lossy(&output.stdout).trim()
    ).ok()?;
    let mut val = &parsed;
    for key in path { val = val.get(*key)?; }
    val.as_str().map(|s| s.to_string())
}

fn read_json_field_from_file(path_str: &str, fields: &[&str]) -> Option<String> {
    let expanded = if let Some(rest) = path_str.strip_prefix("~/") {
        dirs::home_dir()?.join(rest)
    } else {
        PathBuf::from(path_str)
    };
    let data = fs::read_to_string(expanded).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&data).ok()?;
    let mut val = &parsed;
    for key in fields { val = val.get(*key)?; }
    val.as_str().map(|s| s.to_string())
}

fn format_remaining_from_percent(pct: u32, label: &str) -> String {
    let total_hours = match label {
        "5h" => 5.0,
        "wk" => 35.0,
        "24h" => 24.0,
        _ => return format!("{}%", 100 - pct),
    };
    let remaining_hours = total_hours * (1.0 - pct as f64 / 100.0);
    let h = remaining_hours as u64;
    let m = ((remaining_hours - h as f64) * 60.0) as u64;
    if h >= 24 { format!("{}d{}h", h / 24, h % 24) }
    else { format!("{h}h{m:02}m") }
}

// ── Caching ──────────────────────────────────────────────────────────────

fn cache_path(cli: &str) -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(format!(".mtt/cache-{cli}.json")))
}

fn read_cache(cli: &str) -> Option<CLIUsage> {
    let path = cache_path(cli)?;
    let data = fs::read_to_string(&path).ok()?;
    let p: serde_json::Value = serde_json::from_str(&data).ok()?;
    if now_epoch().saturating_sub(p.get("ts")?.as_u64()?) > CACHE_TTL_SECS { return None; }
    Some(CLIUsage {
        primary_percent: p.get("p")?.as_u64()? as u32,
        secondary_percent: p.get("s").and_then(|v| v.as_u64()).map(|v| v as u32),
        primary_label: p.get("pl")?.as_str()?.to_string(),
        secondary_label: p.get("sl").and_then(|v| v.as_str()).unwrap_or("").to_string(),
    })
}

fn write_cache(cli: &str, u: &CLIUsage) {
    if let Some(path) = cache_path(cli) {
        let sec = u.secondary_percent.map(|v| format!(",\"s\":{v}")).unwrap_or_default();
        let json = format!(
            r#"{{"ts":{},"p":{},"pl":"{}","sl":"{}"{sec}}}"#,
            now_epoch(), u.primary_percent, u.primary_label, u.secondary_label
        );
        let _ = fs::write(&path, json);
    }
}

fn now_epoch() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}
