#!/bin/sh
# mtt installer — downloads a prebuilt binary from GitHub Releases.
# Usage: curl -sSf https://raw.githubusercontent.com/kickthemoon0817/mother-terminal/main/install.sh | sh
set -eu

REPO="kickthemoon0817/mother-terminal"
INSTALL_DIR="${MTT_INSTALL_DIR:-$HOME/.mtt/bin}"

info()  { printf '\033[1;34m%s\033[0m\n' "$*"; }
error() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

# ── Detect platform ──────────────────────────────────────────────────
detect_platform() {
    OS=$(uname -s)
    ARCH=$(uname -m)

    case "$OS" in
        Linux)  OS_TAG="linux"  ;;
        Darwin) OS_TAG="darwin" ;;
        *)      error "unsupported OS: $OS" ;;
    esac

    case "$ARCH" in
        x86_64|amd64)   ARCH_TAG="x86_64" ;;
        aarch64|arm64)  ARCH_TAG="arm64"   ;;
        *)              error "unsupported architecture: $ARCH" ;;
    esac

    BINARY_NAME="mtt-${OS_TAG}-${ARCH_TAG}"
}

# ── Fetch latest release tag ─────────────────────────────────────────
get_latest_version() {
    VERSION=$(curl -sSf "https://api.github.com/repos/${REPO}/releases/latest" \
        | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"//;s/".*//')

    if [ -z "$VERSION" ]; then
        error "could not determine latest version"
    fi
}

# ── Download and verify ──────────────────────────────────────────────
download_and_verify() {
    TMPDIR=$(mktemp -d)
    trap 'rm -rf "$TMPDIR"' EXIT

    BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"

    info "downloading mtt ${VERSION} for ${OS_TAG}/${ARCH_TAG}..."

    # Download binary
    curl -sSfL "${BASE_URL}/${BINARY_NAME}" -o "${TMPDIR}/${BINARY_NAME}" \
        || error "failed to download binary"

    # Download checksums
    curl -sSfL "${BASE_URL}/mtt-checksums.sha256" -o "${TMPDIR}/mtt-checksums.sha256" \
        || error "failed to download checksums"

    # Verify checksum
    info "verifying checksum..."
    EXPECTED=$(grep "${BINARY_NAME}" "${TMPDIR}/mtt-checksums.sha256" | awk '{print $1}')
    if [ -z "$EXPECTED" ]; then
        error "no checksum found for ${BINARY_NAME}"
    fi

    if command -v sha256sum >/dev/null 2>&1; then
        ACTUAL=$(sha256sum "${TMPDIR}/${BINARY_NAME}" | awk '{print $1}')
    elif command -v shasum >/dev/null 2>&1; then
        ACTUAL=$(shasum -a 256 "${TMPDIR}/${BINARY_NAME}" | awk '{print $1}')
    else
        error "no sha256sum or shasum found — cannot verify download"
    fi

    if [ "$EXPECTED" != "$ACTUAL" ]; then
        error "checksum mismatch!\n  expected: ${EXPECTED}\n  actual:   ${ACTUAL}\n\nThe download may be corrupted or tampered with. Aborting."
    fi

    info "checksum verified."
}

# ── Install ──────────────────────────────────────────────────────────
install_binary() {
    mkdir -p "$INSTALL_DIR"
    cp "${TMPDIR}/${BINARY_NAME}" "${INSTALL_DIR}/mtt"
    chmod +x "${INSTALL_DIR}/mtt"
    info "installed mtt to ${INSTALL_DIR}/mtt"
}

# ── Update PATH ──────────────────────────────────────────────────────
update_path() {
    # Skip if already in PATH
    case ":$PATH:" in
        *":${INSTALL_DIR}:"*) return ;;
    esac

    SHELL_NAME=$(basename "${SHELL:-/bin/sh}")
    case "$SHELL_NAME" in
        zsh)  RC_FILE="$HOME/.zshrc"  ;;
        bash) RC_FILE="$HOME/.bashrc" ;;
        *)    RC_FILE="" ;;
    esac

    if [ -n "$RC_FILE" ]; then
        EXPORT_LINE="export PATH=\"${INSTALL_DIR}:\$PATH\""
        if [ -f "$RC_FILE" ] && grep -qF "$INSTALL_DIR" "$RC_FILE" 2>/dev/null; then
            return
        fi
        printf '\n# mtt\n%s\n' "$EXPORT_LINE" >> "$RC_FILE"
        info "added ${INSTALL_DIR} to PATH in ${RC_FILE}"
    fi
}

# ── Main ─────────────────────────────────────────────────────────────
main() {
    detect_platform
    get_latest_version
    download_and_verify
    install_binary
    update_path

    printf '\n'
    info "mtt ${VERSION} installed successfully!"
    printf '\n'
    printf '  To get started, restart your shell or run:\n'
    printf '    export PATH="%s:$PATH"\n' "$INSTALL_DIR"
    printf '\n'
    printf '  Then run:\n'
    printf '    mtt\n'
    printf '\n'
}

main
