#!/usr/bin/env bash
# Download sidecar binaries (yt-dlp + demucs-rs) and rename with Tauri target triple.
# Run once after cloning.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="$HERE/../src-tauri/binaries"
mkdir -p "$BIN_DIR"

# Detect host target triple
TRIPLE="$(rustc -vV | sed -n 's|host: ||p')"
echo "Host target: $TRIPLE"

DEMUCS_VERSION="v0.3.4"

# ---- yt-dlp ----
YTDLP_OUT="$BIN_DIR/yt-dlp-$TRIPLE"
case "$TRIPLE" in
  aarch64-apple-darwin | x86_64-apple-darwin)
    YTDLP_URL="https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_macos"
    ;;
  x86_64-unknown-linux-gnu | aarch64-unknown-linux-gnu)
    YTDLP_URL="https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp"
    ;;
  x86_64-pc-windows-msvc)
    YTDLP_URL="https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"
    YTDLP_OUT="$BIN_DIR/yt-dlp-$TRIPLE.exe"
    ;;
  *)
    echo "Unsupported target for yt-dlp: $TRIPLE"; exit 1;;
esac
echo "Downloading yt-dlp → $YTDLP_OUT"
curl -L --fail -o "$YTDLP_OUT" "$YTDLP_URL"
chmod +x "$YTDLP_OUT"

# ---- demucs-rs ----
DEMUCS_OUT="$BIN_DIR/demucs-$TRIPLE"
case "$TRIPLE" in
  aarch64-apple-darwin)
    DEMUCS_ASSET="demucs-aarch64-apple-darwin.tar.gz";;
  x86_64-apple-darwin)
    DEMUCS_ASSET="demucs-x86_64-apple-darwin.tar.gz";;
  x86_64-unknown-linux-gnu)
    DEMUCS_ASSET="demucs-x86_64-unknown-linux-gnu.tar.gz";;
  aarch64-unknown-linux-gnu)
    DEMUCS_ASSET="demucs-aarch64-unknown-linux-gnu.tar.gz";;
  x86_64-pc-windows-msvc)
    DEMUCS_ASSET="demucs-x86_64-pc-windows-msvc.zip"
    DEMUCS_OUT="$BIN_DIR/demucs-$TRIPLE.exe"
    ;;
  *)
    echo "Unsupported target for demucs: $TRIPLE"; exit 1;;
esac

DEMUCS_URL="https://github.com/nikhilunni/demucs-rs/releases/download/$DEMUCS_VERSION/$DEMUCS_ASSET"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "Downloading demucs → $DEMUCS_OUT"
curl -L --fail -o "$TMP/demucs.archive" "$DEMUCS_URL"
if [[ "$DEMUCS_ASSET" == *.tar.gz ]]; then
  tar -xzf "$TMP/demucs.archive" -C "$TMP"
  # Find extracted binary named "demucs"
  EXTRACTED="$(find "$TMP" -type f -name demucs | head -n1)"
else
  unzip -o "$TMP/demucs.archive" -d "$TMP"
  EXTRACTED="$(find "$TMP" -type f -name 'demucs.exe' | head -n1)"
fi
if [[ -z "$EXTRACTED" ]]; then
  echo "Could not locate extracted demucs binary"; exit 1
fi
mv "$EXTRACTED" "$DEMUCS_OUT"
chmod +x "$DEMUCS_OUT"

echo "Binaries done."
ls -la "$BIN_DIR"

echo ""
echo "Next step: build the 'aligner' sidecar (bundles stable_whisper via PyInstaller)."
echo "  ./scripts/build-aligner.sh"
echo "Done."
