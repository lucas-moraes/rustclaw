#!/bin/bash
# Local install: build --release and install to ~/.local/bin (no CI needed).
set -euo pipefail

cd "$(dirname "$0")/.."

log() { printf '%s\n' "$*"; }

log "building rustclaw (release)..."
cargo build --release

BIN="./target/release/rustclaw"
[ -f "$BIN" ] || { log "error: $BIN not found after build"; exit 1; }

DEST="${RUSTCLAW_INSTALL_DIR:-$HOME/.local/bin}"
mkdir -p "$DEST"
install -m 755 "$BIN" "$DEST/rustclaw"
log "installed: $DEST/rustclaw"

case ":$PATH:" in
  *":$DEST:"*) log "✓ run 'rustclaw --version' (already on PATH; new shells see it right away)" ;;
  *)
    log "note: $DEST is not on your PATH."
    log "add it, e.g. in your rc file: export PATH=\"$DEST:\$PATH\""
    exit 1
    ;;
esac
