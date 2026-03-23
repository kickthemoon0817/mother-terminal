use log::{debug, warn};
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

const CACHE_TTL_SECS: u64 = 300; // 5 minutes — avoids rate limits on token refresh

// ── Claude Usage (Anthropic OAuth API) ───────────────────────────────────

const CLAUDE_OAUTH_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";

pub fn fetch_claude_usage() -> Option<CLIUsage> {
    if let Some(cached) = read_cache("claude") {
        debug!("claude usage: from cache");
        return Some(cached);
    }

    debug!("claude usage: fetching from API");
    let refresh = read_json_field_from_keychain("Claude Code-credentials", &["claudeAiOauth", "refreshToken"])
        .or_else(|| read_json_field_from_file("~/.claude/.credentials.json", &["claudeAiOauth", "refreshToken"]));
    if refresh.is_none() {
        warn!("claude usage: no refresh token found in keychain or credentials file");
        return None;
    }
    let refresh = refresh?;

    let body = format!(r#"{{"grant_type":"refresh_token","refresh_token":"{refresh}","client_id":"{CLAUDE_OAUTH_CLIENT_ID}"}}"#);
    let token_resp = curl_post_json("https://platform.claude.com/v1/oauth/token", &body, None);
    if token_resp.is_none() {
        warn!("claude usage: token refresh request failed (curl error)");
        return None;
    }
    let token_resp = token_resp.unwrap();
    debug!("claude usage: refresh response: {}", token_resp);
    let token = token_resp.get("access_token").and_then(|t| t.as_str()).map(|s| s.to_string());
    if token.is_none() {
        warn!("claude usage: no access_token in refresh response");
        return None;
    }
    let token = token?;
    debug!("claude usage: got access token");

    let resp = curl_get("https://api.anthropic.com/api/oauth/usage", &token);
    if resp.is_none() {
        warn!("claude usage: API call failed");
        return None;
    }
    let resp = resp?;
    if resp.get("error").is_some() {
        warn!("claude usage: API returned error: {:?}", resp.get("error"));
        return None;
    }
    debug!("claude usage: API response ok");

    let five = (resp.get("five_hour")?.get("utilization")?.as_f64()? * 100.0).min(100.0) as u32;
    let week = (resp.get("seven_day")?.get("utilization")?.as_f64()? * 100.0).min(100.0) as u32;

    let usage = CLIUsage {
        primary_percent: five, secondary_percent: Some(week),
        primary_label: "5h".into(), secondary_label: "wk".into(),
    };
    write_cache("claude", &usage);
    Some(usage)
}

// ── Codex Usage (parsed from screen) ─────────────────────────────────────

pub fn fetch_codex_usage() -> Option<CLIUsage> {
    if let Some(cached) = read_cache("codex") { debug!("codex usage: from cache"); return Some(cached); }
    // No public API — usage parsed from screen in parse_usage_from_screen()
    debug!("codex usage: no API, relies on screen parsing");
    None
}

/// Parse usage info from a pane's screen text.
/// Recognizes patterns from Claude, Codex, and Gemini status lines.
pub fn parse_usage_from_screen(cli: &str, screen_text: &str) -> Option<CLIUsage> {
    match cli {
        "claude" => parse_claude_screen(screen_text),
        "codex" => parse_codex_screen(screen_text),
        "gemini" => parse_gemini_screen(screen_text),
        _ => None,
    }
}

/// Parse Claude's OMC status line: "5h:16%(3h3m) wk:42%(4d11h)"
fn parse_claude_screen(text: &str) -> Option<CLIUsage> {
    for line in text.lines().rev() {
        // Look for "Xh:XX%(" pattern
        if let Some(pos) = line.find("h:")
            && pos > 0 {
                let before = &line[..pos];
                let digit_start = before.rfind(|c: char| !c.is_ascii_digit()).map(|p| p + 1).unwrap_or(0);
                let after_h = &line[pos + 2..];

                // Parse primary: XX%(
                if let Some(pct_end) = after_h.find('%') {
                    let primary_pct: u32 = after_h[..pct_end].parse().ok()?;

                    // Look for weekly: wk:XX%(
                    let secondary = if let Some(wk_pos) = line.find("wk:") {
                        let wk_after = &line[wk_pos + 3..];
                        if let Some(wk_pct_end) = wk_after.find('%') {
                            wk_after[..wk_pct_end].parse().ok()
                        } else { None }
                    } else { None };

                    let _label = &line[digit_start..pos];
                    return Some(CLIUsage {
                        primary_percent: primary_pct,
                        secondary_percent: secondary,
                        primary_label: "5h".into(),
                        secondary_label: "wk".into(),
                    });
                }
            }
    }
    None
}

/// Parse Codex status line: "XX% left" or "gpt-5.4 low · 99% left"
fn parse_codex_screen(text: &str) -> Option<CLIUsage> {
    for line in text.lines().rev() {
        if let Some(pos) = line.find("% left") {
            // Walk backwards to find the number
            let before = line[..pos].trim_end();
            let num_start = before.rfind(|c: char| !c.is_ascii_digit()).map(|p| p + 1).unwrap_or(0);
            let pct_left: u32 = before[num_start..].parse().ok()?;
            let pct_used = 100u32.saturating_sub(pct_left);
            return Some(CLIUsage {
                primary_percent: pct_used,
                secondary_percent: None,
                primary_label: "5h".into(),
                secondary_label: String::new(),
            });
        }
    }
    None
}

/// Parse Gemini /stats session output: "gemini-2.5-pro    -    ▬▬▬▬▬    XX%  3:16 AM (24h)"
fn parse_gemini_screen(text: &str) -> Option<CLIUsage> {
    let mut max_used: u32 = 0;
    let mut found = false;
    for line in text.lines() {
        // Look for "XX%  HH:MM" pattern in model stats
        if line.contains("gemini-") || line.contains("Gemini") {
            // Find XX% pattern
            for word in line.split_whitespace() {
                if word.ends_with('%')
                    && let Ok(pct) = word.trim_end_matches('%').parse::<u32>() {
                        if pct > max_used { max_used = pct; }
                        found = true;
                    }
            }
        }
    }
    if found {
        Some(CLIUsage {
            primary_percent: max_used,
            secondary_percent: None,
            primary_label: "24h".into(),
            secondary_label: String::new(),
        })
    } else {
        None
    }
}

// ── Gemini Usage (Google Cloud Code Assist API) ──────────────────────────

pub fn fetch_gemini_usage() -> Option<CLIUsage> {
    if let Some(cached) = read_cache("gemini") { debug!("gemini usage: from cache"); return Some(cached); }
    debug!("gemini usage: fetching");

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
        Some(u) => {
            let s = u.format();
            debug!("usage for {cli}: {s}");
            s
        }
        None => {
            debug!("usage for {cli}: unavailable (—)");
            "—".to_string()
        }
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
