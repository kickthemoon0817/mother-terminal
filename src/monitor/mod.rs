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
    /// Not enough time has passed to determine stall status.
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
    pub fn new() -> Self {
        Self::with_timeout(Duration::from_secs(120))
    }

    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            timeout,
            snapshots: HashMap::new(),
        }
    }

    pub fn check(&mut self, name: &str, current_screen_text: &str) -> StallStatus {
        let now = Instant::now();

        match self.snapshots.get_mut(name) {
            None => {
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

    pub fn remove(&mut self, name: &str) {
        self.snapshots.remove(name);
    }
}

impl Default for StallDetector {
    fn default() -> Self {
        Self::new()
    }
}
