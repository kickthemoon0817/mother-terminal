use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// A recorded usage session (start time + duration in seconds).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageEntry {
    pub cli: String,
    pub start_epoch: u64,
    pub duration_secs: u64,
}

/// Tracks usage per CLI type with persistence.
#[derive(Debug, Serialize, Deserialize)]
pub struct UsageTracker {
    entries: Vec<UsageEntry>,
    #[serde(skip)]
    active: HashMap<String, u64>,
}

/// Real usage data from Claude's API (via OMC cache).
#[derive(Debug, Clone, Default)]
pub struct ClaudeUsage {
    pub five_hour_percent: u32,
    pub weekly_percent: u32,
}

impl UsageTracker {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            active: HashMap::new(),
        }
    }

    pub fn load() -> Self {
        let path = match usage_path() {
            Ok(p) => p,
            Err(_) => return Self::new(),
        };
        let data = match fs::read_to_string(&path) {
            Ok(d) => d,
            Err(_) => return Self::new(),
        };
        serde_json::from_str(&data).unwrap_or_else(|_| Self::new())
    }

    pub fn save(&self) -> Result<()> {
        let path = usage_path()?;
        let json = serde_json::to_string_pretty(self).context("serialize usage")?;
        fs::write(&path, &json).context("write usage")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    pub fn start_session(&mut self, cli: &str) {
        let now = now_epoch();
        self.active.insert(cli.to_string(), now);
    }

    pub fn end_session(&mut self, cli: &str) {
        if let Some(start) = self.active.remove(cli) {
            let now = now_epoch();
            let duration = now.saturating_sub(start);
            self.entries.push(UsageEntry {
                cli: cli.to_string(),
                start_epoch: start,
                duration_secs: duration,
            });
        }
    }

    pub fn end_all(&mut self) {
        let active: Vec<String> = self.active.keys().cloned().collect();
        for cli in active {
            self.end_session(&cli);
        }
    }

    pub fn today_secs(&self, cli: &str) -> u64 {
        let today_start = today_start_epoch();
        let mut total: u64 = 0;

        for entry in &self.entries {
            if entry.cli == cli && entry.start_epoch >= today_start {
                total += entry.duration_secs;
            }
        }

        if let Some(&start) = self.active.get(cli) {
            if start >= today_start {
                total += now_epoch().saturating_sub(start);
            } else {
                total += now_epoch().saturating_sub(today_start);
            }
        }

        total
    }

    pub fn week_secs(&self, cli: &str) -> u64 {
        let week_start = week_start_epoch();
        let mut total: u64 = 0;

        for entry in &self.entries {
            if entry.cli == cli && entry.start_epoch >= week_start {
                total += entry.duration_secs;
            }
        }

        if let Some(&start) = self.active.get(cli) {
            if start >= week_start {
                total += now_epoch().saturating_sub(start);
            } else {
                total += now_epoch().saturating_sub(week_start);
            }
        }

        total
    }

    /// Format usage for display: "2h09m today 12h35m week"
    pub fn format_usage(&self, cli: &str) -> String {
        let daily = self.today_secs(cli);
        let weekly = self.week_secs(cli);

        let dh = daily / 3600;
        let dm = (daily % 3600) / 60;
        let wh = weekly / 3600;
        let wm = (weekly % 3600) / 60;

        if weekly > 0 {
            format!("{dh}h{dm:02}m today {wh}h{wm:02}m week")
        } else if daily > 0 {
            format!("{dh}h{dm:02}m today")
        } else {
            "0m".to_string()
        }
    }

    pub fn prune_old(&mut self) {
        let cutoff = now_epoch().saturating_sub(7 * 86400);
        self.entries.retain(|e| e.start_epoch >= cutoff);
    }
}

const OAUTH_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const USAGE_CACHE_TTL_SECS: u64 = 60; // Cache for 60 seconds to avoid rate limits

/// Read OAuth credentials (refresh token) from macOS Keychain or file fallback.
fn get_refresh_token() -> Option<String> {
    // Try macOS Keychain
    if let Ok(output) = std::process::Command::new("security")
        .args(["find-generic-password", "-s", "Claude Code-credentials", "-w"])
        .output()
        && output.status.success() {
            let json_str = String::from_utf8_lossy(&output.stdout);
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_str.trim())
                && let Some(token) = parsed
                    .get("claudeAiOauth")
                    .and_then(|o| o.get("refreshToken"))
                    .and_then(|t| t.as_str())
                {
                    return Some(token.to_string());
                }
        }

    // Fallback: ~/.claude/.credentials.json
    let home = dirs::home_dir()?;
    let cred_path = home.join(".claude/.credentials.json");
    let data = fs::read_to_string(cred_path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&data).ok()?;
    parsed
        .get("claudeAiOauth")
        .and_then(|o| o.get("refreshToken"))
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
}

/// Refresh the access token using the refresh token.
fn refresh_access_token(refresh_token: &str) -> Option<String> {
    let body = format!(
        r#"{{"grant_type":"refresh_token","refresh_token":"{}","client_id":"{}"}}"#,
        refresh_token, OAUTH_CLIENT_ID
    );

    let output = std::process::Command::new("curl")
        .args([
            "-s", "-X", "POST",
            "-H", "Content-Type: application/json",
            "-d", &body,
            "https://platform.claude.com/v1/oauth/token",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let resp: serde_json::Value = serde_json::from_str(
        &String::from_utf8_lossy(&output.stdout)
    ).ok()?;

    resp.get("access_token")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
}

/// Fetch real Claude usage from Anthropic's OAuth API with caching.
pub fn read_claude_usage() -> Option<ClaudeUsage> {
    // Check cache first
    if let Some(cached) = read_usage_cache() {
        return Some(cached);
    }

    // Get fresh data
    let refresh_token = get_refresh_token()?;
    let access_token = refresh_access_token(&refresh_token)?;

    let output = std::process::Command::new("curl")
        .args([
            "-s",
            "-H", &format!("Authorization: Bearer {access_token}"),
            "https://api.anthropic.com/api/oauth/usage",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let body = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&body).ok()?;

    // Check for API errors
    if parsed.get("error").is_some() {
        return None;
    }

    let five_hour = parsed.get("five_hour")?.get("utilization")?.as_f64()?;
    let seven_day = parsed.get("seven_day")?.get("utilization")?.as_f64()?;

    let usage = ClaudeUsage {
        five_hour_percent: (five_hour * 100.0).min(100.0) as u32,
        weekly_percent: (seven_day * 100.0).min(100.0) as u32,
    };

    // Cache the result
    write_usage_cache(&usage);

    Some(usage)
}

fn usage_cache_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(home.join(".mtt/usage-cache.json"))
}

fn read_usage_cache() -> Option<ClaudeUsage> {
    let path = usage_cache_path()?;
    let data = fs::read_to_string(&path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&data).ok()?;

    let ts = parsed.get("timestamp")?.as_u64()?;
    let now = now_epoch();
    if now.saturating_sub(ts) > USAGE_CACHE_TTL_SECS {
        return None; // Cache expired
    }

    Some(ClaudeUsage {
        five_hour_percent: parsed.get("five_hour_percent")?.as_u64()? as u32,
        weekly_percent: parsed.get("weekly_percent")?.as_u64()? as u32,
    })
}

fn write_usage_cache(usage: &ClaudeUsage) {
    if let Some(path) = usage_cache_path() {
        let json = format!(
            r#"{{"timestamp":{},"five_hour_percent":{},"weekly_percent":{}}}"#,
            now_epoch(), usage.five_hour_percent, usage.weekly_percent
        );
        let _ = fs::write(&path, json);
    }
}

/// Format Claude usage from real API data.
pub fn format_claude_api_usage() -> String {
    match read_claude_usage() {
        Some(u) => {
            let five_rem = format_remaining_from_percent(u.five_hour_percent, 5.0);
            let week_rem = format_remaining_from_percent(u.weekly_percent, 35.0);
            format!("5h:{}%({}) wk:{}%({})", u.five_hour_percent, five_rem, u.weekly_percent, week_rem)
        }
        None => "—".to_string(),
    }
}

fn format_remaining_from_percent(pct: u32, total_hours: f64) -> String {
    let remaining_hours = total_hours * (1.0 - pct as f64 / 100.0);
    let h = remaining_hours as u64;
    let m = ((remaining_hours - h as f64) * 60.0) as u64;
    if h >= 24 {
        let d = h / 24;
        let rh = h % 24;
        format!("{d}d{rh}h")
    } else {
        format!("{h}h{m:02}m")
    }
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn today_start_epoch() -> u64 {
    let now = now_epoch();
    now - (now % 86400)
}

fn week_start_epoch() -> u64 {
    let now = now_epoch();
    let day_of_week = (now / 86400 + 4) % 7;
    now - (now % 86400) - (day_of_week * 86400)
}

fn usage_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("no home dir")?;
    let dir = home.join(".mtt");
    fs::create_dir_all(&dir)?;
    Ok(dir.join("usage.json"))
}
