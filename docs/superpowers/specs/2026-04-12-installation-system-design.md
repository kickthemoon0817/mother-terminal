# mtt Installation & Self-Update System

## Goal

Enable users to install mtt with a single command (`curl -sSf ... | sh`) and update it with `mtt update`, without requiring Rust or any build toolchain. All downloads are checksum-verified.

## Platforms

- macOS arm64 (Apple Silicon)
- macOS x86_64 (Intel)
- Linux x86_64
- Linux arm64

## CI: Release Workflow

**Trigger:** Tag push `v*` only.

**Runners (cost-optimized, 2 total):**

| Runner | Native build | Cross-compile |
|--------|-------------|---------------|
| `ubuntu-latest` | linux-x86_64 | linux-arm64 |
| `macos-latest` (arm64) | darwin-arm64 | darwin-x86_64 |

**Security hardening:**
- All actions pinned to commit SHAs
- Minimal permissions: `contents: write` only on release job
- Build with `cargo build --release --locked`
- Generate `mtt-checksums.sha256` from all binaries
- Attach GitHub Artifact Attestations via `actions/attest-build-provenance`
- Create GitHub Release with binaries + checksum file

**Binary naming:** `mtt-<os>-<arch>` (e.g., `mtt-darwin-arm64`, `mtt-linux-x86_64`)

## Install Script (`install.sh`)

Lives at repo root. Invoked via:
```
curl -sSf https://raw.githubusercontent.com/kickthemoon0817/mother-terminal/main/install.sh | sh
```

**Behavior:**
1. Detect OS (`uname -s`) and arch (`uname -m`)
2. Map to binary name (e.g., `Darwin` + `arm64` → `mtt-darwin-arm64`)
3. Fetch latest release tag from GitHub API
4. Download binary + `mtt-checksums.sha256` to temp dir
5. Verify SHA256 checksum — abort if mismatch
6. Install to `${MTT_INSTALL_DIR:-$HOME/.mtt/bin}/mtt`
7. Make executable (`chmod +x`)
8. Add `~/.mtt/bin` to PATH in shell config (`~/.zshrc`, `~/.bashrc`) if not present
9. Print success message with version and PATH instructions

**Fail-safe:** Any download or verification failure aborts with a clear error. Never installs an unverified binary.

## Self-Update (`mtt update`)

New subcommand added to the mtt binary.

**Behavior:**
1. Query GitHub releases API for latest release tag
2. Compare against compiled-in version (`env!("CARGO_PKG_VERSION")`)
3. If current, print "already up to date"
4. If newer available:
   a. Download binary + checksums to temp dir
   b. Verify SHA256 checksum
   c. Atomic rename: write to temp path, verify, rename over current binary
   d. Print before/after version

**Security:** Same checksum verification as install. Never overwrites with unverified binary.

## Startup Version Check

On mtt launch, check for updates in a background thread (non-blocking).

- Cache result in `~/.mtt/version-check.json` with 24h TTL
- If newer version exists, print one-line notice:
  `mtt v0.0.24 available — run mtt update`
- Never blocks startup

## Dependencies

- `ureq` — lightweight HTTP client (no async runtime, small footprint)
- `sha2` — SHA256 computation for checksum verification
- `self_replace` — safe self-binary replacement (or manual atomic rename)

## Files

| File | Purpose |
|------|---------|
| `.github/workflows/release.yml` | CI: build + release |
| `install.sh` | User-facing install script |
| `src/update.rs` | Update checker + self-update logic |
| `src/main.rs` | Wire subcommand + startup check |
| `Cargo.toml` | New dependencies |
