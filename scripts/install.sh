#!/bin/bash
# RustClaw installer (opencode-style).
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/lucas-moraes/rustclaw/main/scripts/install.sh | bash
#
# Env options:
#   RUSTCLAW_VERSION=v0.2.0   pin a specific release tag
#   RUSTCLAW_INSTALL_DIR      install prefix (default: ~/.local/bin)
#   AGENT_DESKTOP_VERSION     agent-desktop release tag (default: v1.2.0)
#   RUSTCLAW_SKIP_AGENT_DESKTOP=1  do not install agent-desktop alongside rustclaw
#
# agent-desktop (vercel-labs) is installed next to rustclaw. A failure there is
# a warning only: rustclaw is still installed.

set -euo pipefail

REPO="lucas-moraes/rustclaw"
BIN="rustclaw"
VERSION="${RUSTCLAW_VERSION:-}"
INSTALL_DIR="${RUSTCLAW_INSTALL_DIR:-$HOME/.local/bin}"
AGENT_DESKTOP_REPO="vercel-labs/agent-desktop"
AGENT_DESKTOP_VERSION="${AGENT_DESKTOP_VERSION:-v1.2.0}"

log()  { printf '%s\n' "$*"; }
die()  { printf 'error: %s\n' "$*" >&2; exit 1; }

# Temp dir for downloads; set inside main(), cleaned on exit.
TMPDIR_INSTALL=""

detect_target() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os" in
    Darwin) os="darwin" ;;
    Linux)  die "Linux is not supported yet (only macOS arm64 is). Build from source: cargo install --path ." ;;
    *) die "unsupported OS: $os (supported: macOS arm64)" ;;
  esac
  case "$arch" in
    arm64|aarch64) arch="arm64" ;;
    *) die "unsupported architecture: $arch (supported: macOS arm64)" ;;
  esac
  printf '%s-%s' "$os" "$arch"
}

download() {
  local url="$1" dest="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL -o "$dest" "$url" || return 1
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$dest" "$url" || return 1
  else
    die "need curl or wget to download"
  fi
}

# Install agent-desktop (vercel-labs) next to rustclaw.
# Best-effort: a failure here warns but does not abort the rustclaw install.
install_agent_desktop() {
  local target="$1" tmp="$2"
  [ "${RUSTCLAW_SKIP_AGENT_DESKTOP:-0}" = "1" ] && { log "skipping agent-desktop (RUSTCLAW_SKIP_AGENT_DESKTOP=1)"; return 0; }

  # Asset names use the full version and a Rust target triple, e.g.
  # agent-desktop-v1.2.0-aarch64-apple-darwin.tar.gz.
  local triple
  case "$target" in
    darwin-arm64) triple="aarch64-apple-darwin" ;;
    *) log "warning: no agent-desktop asset for target $target"; return 0 ;;
  esac
  local archive="agent-desktop-$AGENT_DESKTOP_VERSION-$triple.tar.gz"
  local base="https://github.com/$AGENT_DESKTOP_REPO/releases/download/$AGENT_DESKTOP_VERSION"

  download "$base/$archive" "$tmp/$archive" || {
    log "warning: agent-desktop download failed ($base/$archive)"
    log "  install manually: npm install -g agent-desktop | brew install agent-desktop"
    return 0
  }
  download "$base/checksums.txt" "$tmp/ad-checksums.txt" \
    || log "warning: agent-desktop checksums.txt unavailable, skipping verification"

  if [ -f "$tmp/ad-checksums.txt" ]; then
    local expected
    expected="$(grep "$archive" "$tmp/ad-checksums.txt" | awk '{print $1}')"
    if [ -n "$expected" ]; then
      echo "$expected  $tmp/$archive" | shasum -a 256 -c - >/dev/null 2>&1 \
        || ( command -v sha256sum >/dev/null && echo "$expected  $tmp/$archive" | sha256sum -c - >/dev/null ) \
        || die "checksum mismatch for $archive"
      log "agent-desktop checksum ok"
    fi
  fi

  tar -xzf "$tmp/$archive" -C "$tmp"
  local bin
  for bin in agent-desktop agent-desktop-macos-helper; do
    [ -f "$tmp/$bin" ] || { log "warning: agent-desktop archive did not contain $bin"; return 0; }
    chmod +x "$tmp/$bin"
    mv "$tmp/$bin" "$INSTALL_DIR/$bin"
  done
  log "✓ installed: $INSTALL_DIR/agent-desktop ($AGENT_DESKTOP_VERSION)"
  log "  grant Accessibility permission: agent-desktop permissions --guide"
}

main() {
  local target
  target="$(detect_target)"

  # Resolve the latest release tag when not pinned.
  if [ -z "$VERSION" ]; then
    local api_url="https://api.github.com/repos/$REPO/releases/latest"
    if command -v curl >/dev/null 2>&1; then
      VERSION="$(curl -fsSL "$api_url" | grep -o '"tag_name": *"[^"]*"' | sed 's/.*"tag_name": *"\(.*\)"/\1/')" || true
    elif command -v wget >/dev/null 2>&1; then
      VERSION="$(wget -qO- "$api_url" | grep -o '"tag_name": *"[^"]*"' | sed 's/.*"tag_name": *"\(.*\)"/\1/')" || true
    fi
    [ -n "$VERSION" ] || die "could not resolve latest release (set RUSTCLAW_VERSION to a tag, e.g. v0.2.0)"
  fi
  log "installing $BIN $VERSION for $target"

  local archive="rustclaw-${target}.tar.gz"
  local base="${RUSTCLAW_DOWNLOAD_BASE:-https://github.com/$REPO/releases/download/$VERSION}"
  TMPDIR_INSTALL="$(mktemp -d)"
  local tmp="$TMPDIR_INSTALL"
  trap 'rm -rf "$TMPDIR_INSTALL"' EXIT

  download "$base/$archive" "$tmp/$archive" \
    || die "download failed: $base/$archive (does the release exist?)"
  download "$base/checksums.txt" "$tmp/checksums.txt" \
    || log "warning: checksums.txt unavailable, skipping verification"

  if [ -f "$tmp/checksums.txt" ]; then
    local expected
    expected="$(grep "$archive" "$tmp/checksums.txt" | awk '{print $1}')"
    if [ -n "$expected" ]; then
      echo "$expected  $tmp/$archive" | shasum -a 256 -c - >/dev/null 2>&1 \
        || ( command -v sha256sum >/dev/null && echo "$expected  $tmp/$archive" | sha256sum -c - >/dev/null ) \
        || die "checksum mismatch for $archive"
      log "checksum ok"
    fi
  fi

  tar -xzf "$tmp/$archive" -C "$tmp"
  [ -f "$tmp/$BIN" ] || die "archive did not contain $BIN"
  chmod +x "$tmp/$BIN"

  mkdir -p "$INSTALL_DIR"
  mv "$tmp/$BIN" "$INSTALL_DIR/$BIN"

  # Ensure the install dir is on PATH (idempotent, no duplicates).
  case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *)
      export PATH="$INSTALL_DIR:$PATH"
      local rc_file
      rc_file="$HOME/.zshrc"
      [ "$SHELL" = "/bin/zsh" ] || rc_file="$HOME/.profile"
      {
        printf '\n# RustClaw (added by install script)\n'
        printf 'export PATH="%s:$PATH"\n' "$INSTALL_DIR"
      } >>"$rc_file"
      log "added $INSTALL_DIR to PATH in $rc_file (restart your shell)"
      ;;
  esac

  log ""
  log "✓ installed: $(command -v "$BIN" || printf '%s/%s' "$INSTALL_DIR" "$BIN")"
  log "  run 'rustclaw --version' to confirm"

  install_agent_desktop "$target" "$tmp"
}

main "$@"
