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
    active: HashMap<String, u64>, // cli -> start_epoch for currently running sessions
}

impl UsageTracker {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            active: HashMap::new(),
        }
    }

    /// Load from disk.
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

    /// Save to disk.
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

    /// Mark a session as started.
    pub fn start_session(&mut self, cli: &str) {
        let now = now_epoch();
        self.active.insert(cli.to_string(), now);
    }

    /// Mark a session as ended and record the duration.
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

    /// End all active sessions (on quit).
    pub fn end_all(&mut self) {
        let active: Vec<String> = self.active.keys().cloned().collect();
        for cli in active {
            self.end_session(&cli);
        }
    }

    /// Get total usage today for a CLI (in seconds).
    pub fn today_secs(&self, cli: &str) -> u64 {
        let today_start = today_start_epoch();
        let mut total: u64 = 0;

        // Recorded entries
        for entry in &self.entries {
            if entry.cli == cli && entry.start_epoch >= today_start {
                total += entry.duration_secs;
            }
        }

        // Currently active session
        if let Some(&start) = self.active.get(cli) {
            if start >= today_start {
                total += now_epoch().saturating_sub(start);
            } else {
                // Started before today, count only today's portion
                total += now_epoch().saturating_sub(today_start);
            }
        }

        total
    }

    /// Get total usage this week for a CLI (in seconds).
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

    /// Format usage for display: "2h09m today  12h35m week"
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

    /// Prune entries older than 7 days to keep file small.
    pub fn prune_old(&mut self) {
        let cutoff = now_epoch().saturating_sub(7 * 86400);
        self.entries.retain(|e| e.start_epoch >= cutoff);
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
    now - (now % 86400) // midnight UTC
}

fn week_start_epoch() -> u64 {
    let now = now_epoch();
    let day_of_week = (now / 86400 + 4) % 7; // 0=Monday
    now - (now % 86400) - (day_of_week * 86400)
}

fn usage_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("no home dir")?;
    let dir = home.join(".mtt");
    fs::create_dir_all(&dir)?;
    Ok(dir.join("usage.json"))
}
