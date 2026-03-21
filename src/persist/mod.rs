use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Snapshot of a single mtt session that can be saved and restored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub cli_type: String,
    pub cwd: String,
    pub status: String,
}

/// Returns the path to `~/.mtt/sessions.json`, creating the directory if needed.
fn sessions_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not determine home directory")?;
    let dir = home.join(".mtt");
    fs::create_dir_all(&dir).with_context(|| format!("could not create {}", dir.display()))?;
    Ok(dir.join("sessions.json"))
}

/// Serialize `sessions` to `~/.mtt/sessions.json` with 0600 permissions.
pub fn save_sessions(sessions: &[SessionInfo]) -> Result<()> {
    let path = sessions_path()?;
    let json = serde_json::to_string_pretty(sessions).context("failed to serialize sessions")?;

    // Write via a temp file so partial writes don't corrupt the saved state.
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, &json)
        .with_context(|| format!("failed to write {}", tmp_path.display()))?;

    // Set 0600 permissions before the atomic rename.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed to set permissions on {}", tmp_path.display()))?;
    }

    fs::rename(&tmp_path, &path)
        .with_context(|| format!("failed to rename {} -> {}", tmp_path.display(), path.display()))?;

    Ok(())
}

/// Load sessions from `~/.mtt/sessions.json`.
/// Returns an empty `Vec` when the file does not exist.
pub fn load_sessions() -> Vec<SessionInfo> {
    let path = match sessions_path() {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };

    let data = match fs::read_to_string(&path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    serde_json::from_str(&data).unwrap_or_default()
}
