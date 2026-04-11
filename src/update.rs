use anyhow::{Context, Result, bail};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

const REPO: &str = "kickthemoon0817/mother-terminal";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const CHECK_CACHE_SECS: u64 = 86400; // 24 hours
const API_TIMEOUT: Duration = Duration::from_secs(10);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_API_RESPONSE: u64 = 1024 * 1024; // 1 MB
const MAX_BINARY_SIZE: u64 = 100 * 1024 * 1024; // 100 MB

/// Cached version check result.
#[derive(serde::Serialize, serde::Deserialize)]
struct VersionCache {
    latest: String,
    checked_at: u64,
}

fn cache_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not determine home directory")?;
    let dir = home.join(".mtt");
    fs::create_dir_all(&dir)?;
    Ok(dir.join("version-check.json"))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn api_agent() -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(API_TIMEOUT))
            .build()
    )
}

fn download_agent() -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(DOWNLOAD_TIMEOUT))
            .build()
    )
}

/// Fetch the latest release tag from GitHub API.
fn fetch_latest_version() -> Result<String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let resp = api_agent()
        .get(&url)
        .header("User-Agent", &format!("mtt/{CURRENT_VERSION}"))
        .call()
        .context("failed to query GitHub releases")?;

    let body_str = resp
        .into_body()
        .with_config()
        .limit(MAX_API_RESPONSE)
        .read_to_string()
        .context("failed to read GitHub API response")?;
    let body: serde_json::Value = serde_json::from_str(&body_str)
        .context("failed to parse GitHub API response")?;

    let tag = body["tag_name"]
        .as_str()
        .context("no tag_name in release response")?
        .trim_start_matches('v')
        .to_string();

    Ok(tag)
}

/// Check for a newer version. Returns Some(latest) if an update is available.
/// Uses a 24-hour cache to avoid hitting the API on every launch.
pub fn check_for_update_cached() -> Option<String> {
    let path = cache_path().ok()?;

    // Try reading cache
    if let Ok(data) = fs::read_to_string(&path)
        && let Ok(cache) = serde_json::from_str::<VersionCache>(&data)
        && now_secs().saturating_sub(cache.checked_at) < CHECK_CACHE_SECS {
            return if is_newer(&cache.latest) {
                Some(cache.latest)
            } else {
                None
            };
        }

    // Cache miss or stale — fetch and update cache
    let latest = fetch_latest_version().ok()?;
    let cache = VersionCache {
        latest: latest.clone(),
        checked_at: now_secs(),
    };
    if let Ok(json) = serde_json::to_string(&cache) {
        let _ = fs::write(&path, json);
    }

    if is_newer(&latest) {
        Some(latest)
    } else {
        None
    }
}

/// Compare version strings. Returns true if `latest` is newer than current.
fn is_newer(latest: &str) -> bool {
    let parse = |v: &str| -> Vec<u32> {
        v.split('.')
            .filter_map(|s| s.parse().ok())
            .collect()
    };
    let current = parse(CURRENT_VERSION);
    let other = parse(latest);

    other > current
}

/// Run the self-update: download the latest binary and replace in place.
pub fn self_update() -> Result<()> {
    println!("checking for updates...");

    let latest = fetch_latest_version()?;

    if !is_newer(&latest) {
        println!("mtt v{CURRENT_VERSION} is already up to date.");
        return Ok(());
    }

    println!("updating mtt v{CURRENT_VERSION} → v{latest}...");

    let binary_name = get_binary_name()?;
    let base_url = format!("https://github.com/{REPO}/releases/download/v{latest}");

    // Use a unique temp directory to prevent symlink/TOCTOU attacks
    let tmp_dir = std::env::temp_dir().join(format!("mtt-update-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&tmp_dir)?;

    let result = do_update(&tmp_dir, &base_url, &binary_name, &latest);

    // Always clean up temp dir
    let _ = fs::remove_dir_all(&tmp_dir);

    result
}

fn do_update(tmp_dir: &std::path::Path, base_url: &str, binary_name: &str, latest: &str) -> Result<()> {
    let tmp_binary = tmp_dir.join("mtt-new");
    let tmp_checksums = tmp_dir.join("mtt-checksums.sha256");

    download_file(&format!("{base_url}/{binary_name}"), &tmp_binary, MAX_BINARY_SIZE)?;
    download_file(&format!("{base_url}/mtt-checksums.sha256"), &tmp_checksums, MAX_API_RESPONSE)?;

    // Verify checksum
    verify_checksum(&tmp_binary, &tmp_checksums, binary_name)?;

    // Replace current binary
    let current_exe = std::env::current_exe()
        .context("could not determine current executable path")?;

    #[cfg(unix)]
    replace_binary(&tmp_binary, &current_exe)?;

    // Update cache
    if let Ok(path) = cache_path() {
        let cache = VersionCache {
            latest: latest.to_string(),
            checked_at: now_secs(),
        };
        if let Ok(json) = serde_json::to_string(&cache) {
            let _ = fs::write(&path, json);
        }
    }

    println!("mtt updated to v{latest}.");
    Ok(())
}

fn get_binary_name() -> Result<String> {
    let os = if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        bail!("unsupported OS for self-update");
    };

    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else {
        bail!("unsupported architecture for self-update");
    };

    Ok(format!("mtt-{os}-{arch}"))
}

fn download_file(url: &str, dest: &std::path::Path, max_size: u64) -> Result<()> {
    let resp = download_agent()
        .get(url)
        .header("User-Agent", &format!("mtt/{CURRENT_VERSION}"))
        .call()
        .with_context(|| format!("failed to download {url}"))?;

    let body_bytes = resp
        .into_body()
        .with_config()
        .limit(max_size)
        .read_to_vec()
        .with_context(|| format!("failed to read response from {url}"))?;
    fs::write(dest, &body_bytes)
        .with_context(|| format!("failed to write {}", dest.display()))?;
    Ok(())
}

fn verify_checksum(binary: &std::path::Path, checksums: &std::path::Path, name: &str) -> Result<()> {
    use sha2::{Sha256, Digest};

    let checksum_content = fs::read_to_string(checksums)
        .context("failed to read checksums file")?;

    // Anchored match: line must end with the exact binary name
    let expected = checksum_content
        .lines()
        .find(|line| line.ends_with(name))
        .and_then(|line| line.split_whitespace().next())
        .context("no matching checksum found for this platform")?;

    let binary_data = fs::read(binary)
        .context("failed to read downloaded binary")?;
    let actual = format!("{:x}", Sha256::digest(&binary_data));

    if expected != actual {
        bail!(
            "checksum mismatch!\n  expected: {expected}\n  actual:   {actual}\n\nThe download may be corrupted or tampered with."
        );
    }

    println!("checksum verified.");
    Ok(())
}

#[cfg(unix)]
fn replace_binary(new: &std::path::Path, current: &std::path::Path) -> Result<()> {
    // Copy permissions from current binary
    let perms = fs::metadata(current)
        .context("failed to read current binary metadata")?
        .permissions();

    // Copy the new binary to a temp file in the SAME directory as current
    // to ensure fs::rename works (same filesystem).
    let staging = current.with_extension("new");
    fs::copy(new, &staging)
        .context("failed to stage new binary")?;
    fs::set_permissions(&staging, perms.clone())
        .context("failed to set permissions on staged binary")?;

    // Rename dance: move current aside, move staged into place
    let backup = current.with_extension("old");
    fs::rename(current, &backup)
        .context("failed to move current binary aside")?;

    if let Err(e) = fs::rename(&staging, current) {
        // Restore backup on failure
        let _ = fs::rename(&backup, current);
        let _ = fs::remove_file(&staging);
        return Err(e).context("failed to install new binary");
    }

    let _ = fs::remove_file(&backup);

    Ok(())
}
