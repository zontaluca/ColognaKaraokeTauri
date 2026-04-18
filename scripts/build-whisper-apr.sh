#!/usr/bin/env bash
# Build whisper-apr sidecar binary (pure Rust, no Python required).
# Clones from GitHub and patches Cargo.toml to remove the broken apr-cli dep
# (apr-cli 0.4.x are all yanked on crates.io; the dep is only used behind
# #[cfg(feature = "cli-full")] anyway, so stripping it from "cli" is safe).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="$HERE/../src-tauri/binaries"
mkdir -p "$BIN_DIR"

TRIPLE="$(rustc -vV | sed -n 's|host: ||p')"
echo "Target triple: $TRIPLE"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "Cloning whisper-apr..."
git clone --depth 1 https://github.com/paiml/whisper.apr "$WORK/whisper-apr" 2>&1

echo "Patching Cargo.toml: removing broken dep:apr-cli from cli feature..."
# Remove ", dep:apr-cli" or "dep:apr-cli, " or standalone "dep:apr-cli" from cli feature line
sed -i.bak 's/, *"dep:apr-cli"//g; s/"dep:apr-cli" *, *//g; s/"dep:apr-cli"//g' \
    "$WORK/whisper-apr/Cargo.toml"

echo "Building whisper-apr..."
cargo build \
    --manifest-path "$WORK/whisper-apr/Cargo.toml" \
    --release \
    --bin whisper-apr \
    --features cli,converter \
    2>&1

case "$TRIPLE" in
  *-windows-*) EXT=".exe" ;;
  *) EXT="" ;;
esac

SRC="$WORK/whisper-apr/target/release/whisper-apr$EXT"
if [[ ! -f "$SRC" ]]; then
  echo "ERROR: binary not found at $SRC after build"
  exit 1
fi

DST="$BIN_DIR/whisper-apr-$TRIPLE$EXT"
cp "$SRC" "$DST"
chmod +x "$DST"

echo ""
echo "whisper-apr sidecar built → $DST"
echo ""
echo "Note: on first song processing, the base Whisper model (~150 MB) will be"
echo "downloaded automatically to the default model cache."
