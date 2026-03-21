use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Result of a stall check for a pane.
#[derive(Debug, Clone, PartialEq)]
pub enum StallStatus {
    /// Output changed since last snapshot — the CLI is still working.
    Active,
    /// No output change for longer than the configured timeout.
    /// The contained bytes are the auto-resume action to send to the pane.
    Stalled { resume_action: Vec<u8> },
    /// Not enough time has passed to determine stall status (first check or
    /// output changed very recently).
    Unchanged,
}

struct ScreenSnapshot {
    text: String,
    last_changed: Instant,
}

/// Detects when a pane has stopped producing output for a configurable window.
pub struct StallDetector {
    timeout: Duration,
    snapshots: HashMap<String, ScreenSnapshot>,
}

impl StallDetector {
    /// Create a detector with the default 120-second timeout.
    pub fn new() -> Self {
        Self::with_timeout(Duration::from_secs(120))
    }

    /// Create a detector with a custom timeout.
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            timeout,
            snapshots: HashMap::new(),
        }
    }

    /// Check a pane identified by `name` against its current screen text.
    ///
    /// - Returns `Active` when the screen text is different from the last snapshot.
    /// - Returns `Stalled` (with a `"continue\r"` resume action) when the text
    ///   has been identical for longer than the configured timeout.
    /// - Returns `Unchanged` when the text matches but the timeout has not yet
    ///   elapsed.
    pub fn check(&mut self, name: &str, current_screen_text: &str) -> StallStatus {
        let now = Instant::now();

        match self.snapshots.get_mut(name) {
            None => {
                // First time we've seen this pane — record and report unchanged.
                self.snapshots.insert(
                    name.to_string(),
                    ScreenSnapshot {
                        text: current_screen_text.to_string(),
                        last_changed: now,
                    },
                );
                StallStatus::Unchanged
            }
            Some(snap) => {
                if snap.text != current_screen_text {
                    // Screen changed — update snapshot and report active.
                    snap.text = current_screen_text.to_string();
                    snap.last_changed = now;
                    StallStatus::Active
                } else if now.duration_since(snap.last_changed) >= self.timeout {
                    StallStatus::Stalled {
                        resume_action: b"continue\r".to_vec(),
                    }
                } else {
                    StallStatus::Unchanged
                }
            }
        }
    }

    /// Remove tracking state for a pane (e.g. when it is killed).
    pub fn remove(&mut self, name: &str) {
        self.snapshots.remove(name);
    }
}

impl Default for StallDetector {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Usage window tracking
// ---------------------------------------------------------------------------

/// The AI CLI type, used to choose the default session window length.
#[derive(Debug, Clone, PartialEq)]
pub enum CliKind {
    Claude,
    Codex,
    Gemini,
    Other,
}

impl CliKind {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "claude" => Self::Claude,
            "codex" => Self::Codex,
            "gemini" => Self::Gemini,
            _ => Self::Other,
        }
    }

    /// Default usage window for this CLI type.
    fn default_window(&self) -> Duration {
        match self {
            Self::Claude | Self::Codex => Duration::from_secs(5 * 3600), // 5 h
            Self::Gemini => Duration::from_secs(4 * 3600),               // 4 h
            Self::Other => Duration::from_secs(5 * 3600),                // 5 h
        }
    }
}

struct UsageWindow {
    started: Instant,
    window: Duration,
}

/// Tracks per-CLI usage windows so callers can see how much quota time remains.
pub struct UsageTracker {
    windows: HashMap<String, UsageWindow>,
}

impl UsageTracker {
    pub fn new() -> Self {
        Self {
            windows: HashMap::new(),
        }
    }

    /// Start (or restart) a usage window for `name` using the default window
    /// duration for `cli_type`.
    pub fn start_window(&mut self, name: &str, cli_type: &CliKind) {
        self.windows.insert(
            name.to_string(),
            UsageWindow {
                started: Instant::now(),
                window: cli_type.default_window(),
            },
        );
    }

    /// Returns the remaining time in the window, or `None` if no window exists
    /// or the window has already expired.
    pub fn remaining(&self, name: &str) -> Option<Duration> {
        let w = self.windows.get(name)?;
        let elapsed = w.started.elapsed();
        if elapsed >= w.window {
            None
        } else {
            Some(w.window - elapsed)
        }
    }

    /// Returns a human-readable remaining time string like `"3h22m"`, or
    /// `"expired"` / `"unknown"` when appropriate.
    pub fn format_remaining(&self, name: &str) -> String {
        match self.remaining(name) {
            None => {
                if self.windows.contains_key(name) {
                    "expired".to_string()
                } else {
                    "unknown".to_string()
                }
            }
            Some(d) => {
                let total_secs = d.as_secs();
                let hours = total_secs / 3600;
                let mins = (total_secs % 3600) / 60;
                match (hours, mins) {
                    (0, m) => format!("{m}m"),
                    (h, 0) => format!("{h}h"),
                    (h, m) => format!("{h}h{m}m"),
                }
            }
        }
    }

    /// Remove tracking state for a session.
    pub fn remove(&mut self, name: &str) {
        self.windows.remove(name);
    }
}

impl Default for UsageTracker {
    fn default() -> Self {
        Self::new()
    }
}
