use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

/// Encode a cwd path the same way Claude Code does: replace `/` with `-`.
fn encode_cwd(cwd: &str) -> String {
    cwd.replace('/', "-")
}

/// Return the Claude projects directory for a given cwd.
fn claude_project_dir(cwd: &str) -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not determine home directory")?;
    let dir = home.join(".claude").join("projects").join(encode_cwd(cwd));
    if !dir.exists() {
        bail!("no Claude sessions found for {}", cwd);
    }
    Ok(dir)
}

/// Find the most recently modified session JSONL file for a cwd.
/// Returns the session ID (UUID filename stem).
pub fn find_latest_session(cwd: &str) -> Result<String> {
    let dir = claude_project_dir(cwd)?;
    let mut entries: Vec<_> = fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext == "jsonl")
        })
        .collect();

    if entries.is_empty() {
        bail!("no session files in {}", dir.display());
    }

    entries.sort_by_key(|e| {
        e.metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });

    let latest = entries
        .last()
        .context("no session files found")?;

    let session_id = latest
        .path()
        .file_stem()
        .context("invalid session filename")?
        .to_string_lossy()
        .to_string();

    Ok(session_id)
}

/// Create a branch of an existing Claude Code session by copying and
/// re-keying the JSONL conversation file. Returns the new session ID.
pub fn create_branch(cwd: &str, base_session_id: &str) -> Result<String> {
    let dir = claude_project_dir(cwd)?;
    let base_path = dir.join(format!("{base_session_id}.jsonl"));

    if !base_path.exists() {
        bail!("session file not found: {}", base_path.display());
    }

    let new_id = Uuid::new_v4().to_string();
    let new_path = dir.join(format!("{new_id}.jsonl"));

    let content = fs::read_to_string(&base_path)
        .with_context(|| format!("failed to read {}", base_path.display()))?;

    let mut output_lines = Vec::new();
    let mut is_first = true;

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }

        let mut record: Value = serde_json::from_str(line)
            .context("failed to parse session record")?;

        if let Some(obj) = record.as_object_mut() {
            obj.insert("sessionId".to_string(), Value::String(new_id.clone()));

            if is_first {
                let fork_info = serde_json::json!({
                    "sessionId": base_session_id,
                    "messageUuid": obj.get("uuid").cloned().unwrap_or(Value::Null),
                });
                obj.insert("forkedFrom".to_string(), fork_info);
                is_first = false;
            }
        }

        output_lines.push(serde_json::to_string(&record)?);
    }

    fs::write(&new_path, output_lines.join("\n"))
        .with_context(|| format!("failed to write {}", new_path.display()))?;

    Ok(new_id)
}
