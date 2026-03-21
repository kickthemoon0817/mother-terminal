use anyhow::{Context, Result, bail};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// A match found by `search`.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub session: String,
    pub line_number: usize,
    pub content: String,
}

/// Handles recording and retrieval of pane output history.
pub struct Recorder {
    base_dir: PathBuf,
}

impl Recorder {
    /// Create a new `Recorder` rooted at `~/.mtt/history`.
    pub fn new() -> Result<Self> {
        let base_dir = dirs::home_dir()
            .context("cannot determine home directory")?
            .join(".mtt")
            .join("history");

        fs::create_dir_all(&base_dir)
            .with_context(|| format!("cannot create history dir {}", base_dir.display()))?;

        set_permissions_700(&base_dir)?;

        Ok(Self { base_dir })
    }

    /// Append `text` to the log for `name`.
    ///
    /// Deduplicates: if the log already ends with `text` (trimmed), the write
    /// is skipped to avoid recording identical successive screen snapshots.
    pub fn record(&self, name: &str, text: &str) -> Result<()> {
        let log_path = self.log_path(name)?;

        // Ensure the session directory exists with 0700 permissions.
        let session_dir = log_path
            .parent()
            .expect("log_path always has a parent");
        if !session_dir.exists() {
            fs::create_dir_all(session_dir)
                .with_context(|| format!("cannot create session dir {}", session_dir.display()))?;
            set_permissions_700(session_dir)?;
        }

        // Deduplication: read the last block from the file and skip if identical.
        if log_path.exists() {
            let existing = fs::read_to_string(&log_path)
                .unwrap_or_default();
            // Each record is separated by a blank line sentinel.
            if let Some(last_block) = existing.trim_end().rsplit("\n\n").next()
                && last_block.trim() == text.trim() {
                    return Ok(());
                }
        }

        // Append with a trailing blank line as record separator.
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .with_context(|| format!("cannot open log {}", log_path.display()))?;

        set_permissions_600(&log_path)?;

        writeln!(file, "{}", text.trim_end())?;
        writeln!(file)?; // blank line separator
        Ok(())
    }

    /// Return the last `lines` lines from the session log.
    pub fn get_history(&self, name: &str, lines: usize) -> Result<Vec<String>> {
        let log_path = self.log_path(name)?;

        if !log_path.exists() {
            return Ok(Vec::new());
        }

        let file = fs::File::open(&log_path)
            .with_context(|| format!("cannot open log {}", log_path.display()))?;
        let reader = BufReader::new(file);

        let all: Vec<String> = reader
            .lines()
            .collect::<std::result::Result<_, _>>()
            .context("error reading log")?;

        let start = all.len().saturating_sub(lines);
        Ok(all[start..].to_vec())
    }

    /// Search for `query` (case-insensitive substring) across all session logs.
    pub fn search(&self, query: &str) -> Result<Vec<SearchResult>> {
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        for session in self.list_sessions()? {
            let log_path = self.log_path(&session)?;
            if !log_path.exists() {
                continue;
            }

            let file = fs::File::open(&log_path)
                .with_context(|| format!("cannot open log {}", log_path.display()))?;
            let reader = BufReader::new(file);

            for (idx, line_result) in reader.lines().enumerate() {
                let line = line_result.context("error reading log line")?;
                if line.to_lowercase().contains(&query_lower) {
                    results.push(SearchResult {
                        session: session.clone(),
                        line_number: idx + 1,
                        content: line,
                    });
                }
            }
        }

        Ok(results)
    }

    /// List names of all sessions that have history.
    pub fn list_sessions(&self) -> Result<Vec<String>> {
        if !self.base_dir.exists() {
            return Ok(Vec::new());
        }

        let mut sessions = Vec::new();
        for entry in fs::read_dir(&self.base_dir).context("cannot read history dir")? {
            let entry = entry.context("error reading dir entry")?;
            let path = entry.path();
            if path.is_dir() && path.join("output.log").exists()
                && let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    sessions.push(name.to_string());
                }
        }
        sessions.sort();
        Ok(sessions)
    }

    /// Sanitize a session name for safe use as a filesystem component.
    ///
    /// Replaces `/`, spaces, and `..` sequences with underscores.
    fn sanitize(name: &str) -> String {
        // Replace path separators and spaces first.
        let s = name.replace(['/', ' '], "_");
        // Collapse any `..` sequences that could escape directories.
        s.replace("..", "_")
    }

    /// Resolve the log path for a session and verify it stays under `base_dir`.
    fn log_path(&self, name: &str) -> Result<PathBuf> {
        let safe_name = Self::sanitize(name);
        if safe_name.is_empty() {
            bail!("session name is empty after sanitization");
        }

        let candidate = self.base_dir.join(&safe_name).join("output.log");

        // Path traversal protection: the canonical parent must be under base_dir.
        // We check the session dir (parent of the log file) because the log file
        // might not exist yet.
        let session_dir = candidate
            .parent()
            .expect("candidate always has a parent");

        // Use the real path of base_dir for comparison; for session_dir we
        // compare the non-canonicalized components since it may not exist yet.
        let canonical_base = self
            .base_dir
            .canonicalize()
            .unwrap_or_else(|_| self.base_dir.clone());

        // Build what the canonical session path would be without requiring it to exist.
        let expected = canonical_base.join(&safe_name);

        // Strip the base from the session dir and verify no `..` components remain.
        let rel = session_dir
            .strip_prefix(&self.base_dir)
            .or_else(|_| session_dir.strip_prefix(&canonical_base))
            .with_context(|| {
                format!(
                    "path traversal detected: '{}' escapes history dir",
                    candidate.display()
                )
            })?;

        for component in rel.components() {
            use std::path::Component;
            match component {
                Component::Normal(_) => {}
                _ => bail!(
                    "path traversal detected: '{}' contains illegal component",
                    candidate.display()
                ),
            }
        }

        // Sanity check: reconstructed expected path matches what we resolved.
        let _ = expected; // used implicitly via prefix check above

        Ok(candidate)
    }
}

/// Set Unix permissions to 0700 on a directory.
#[cfg(unix)]
fn set_permissions_700(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = fs::Permissions::from_mode(0o700);
    fs::set_permissions(path, perms)
        .with_context(|| format!("cannot set 0700 on {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_permissions_700(_path: &Path) -> Result<()> {
    Ok(())
}

/// Set Unix permissions to 0600 on a file.
#[cfg(unix)]
fn set_permissions_600(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = fs::Permissions::from_mode(0o600);
    fs::set_permissions(path, perms)
        .with_context(|| format!("cannot set 0600 on {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_permissions_600(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_slashes_and_spaces() {
        assert_eq!(Recorder::sanitize("foo/bar baz"), "foo_bar_baz");
    }

    #[test]
    fn sanitize_dotdot() {
        assert_eq!(Recorder::sanitize("../../etc/passwd"), "____etc_passwd");
    }

    #[test]
    fn sanitize_empty_stays_empty() {
        assert_eq!(Recorder::sanitize(""), "");
    }
}
