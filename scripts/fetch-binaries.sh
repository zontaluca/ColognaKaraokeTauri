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

# ---- wav2vec2 ONNX models ----
# Layout matches `aligner_wav2vec2::default_local_dir(lang)`:
#   <cache>/cologna-karaoke/wav2vec2/<lang>/{model.onnx,vocab.json}
#
# Only English is hosted as pre-exported ONNX (Xenova/transformers.js). Other
# languages must be exported locally with scripts/export-wav2vec2-onnx.py.
case "$(uname)" in
  Darwin) W2V_CACHE="$HOME/Library/Caches/cologna-karaoke/wav2vec2";;
  Linux)  W2V_CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/cologna-karaoke/wav2vec2";;
  *)      W2V_CACHE="$HOME/.cache/cologna-karaoke/wav2vec2";;
esac

W2V_LANGS="${W2V_LANGS:-en}"  # override with `W2V_LANGS="en it" ./scripts/fetch-binaries.sh`
# pick "fp16" for ~half-size, "bnb4" for ~quarter (slower) — see Xenova onnx/ tree.
W2V_VARIANT="${W2V_VARIANT:-fp32}"
case "$W2V_VARIANT" in
  fp32) ONNX_FILE="model.onnx";;
  fp16) ONNX_FILE="model_fp16.onnx";;
  bnb4) ONNX_FILE="model_bnb4.onnx";;
  *)    echo "[w2v] unknown W2V_VARIANT=$W2V_VARIANT (fp32|fp16|bnb4)"; exit 1;;
esac

w2v_preset_repo() {
  # Bash 3.2-compatible (macOS default) — no associative arrays.
  case "$1" in
    en) echo "Xenova/wav2vec2-large-xlsr-53-english";;
    *)  echo "";;
  esac
}

for lang in $W2V_LANGS; do
  dest="$W2V_CACHE/$lang"
  mkdir -p "$dest"
  if [[ -f "$dest/model.onnx" && -f "$dest/vocab.json" ]]; then
    echo "[w2v] $lang already present at $dest — skip"
    continue
  fi

  repo="$(w2v_preset_repo "$lang")"
  if [[ -z "$repo" ]]; then
    echo "[w2v] no pre-exported ONNX for '$lang'."
    echo "      Export locally with:"
    echo "        pip install transformers optimum[onnxruntime] onnx torch"
    echo "        python3 scripts/export-wav2vec2-onnx.py --langs $lang"
    continue
  fi

  base="https://huggingface.co/$repo/resolve/main"
  echo "[w2v] downloading $lang from $repo → $dest ($W2V_VARIANT)"
  curl -L --fail -o "$dest/vocab.json" "$base/vocab.json" || {
    echo "[w2v] vocab.json fetch failed"; rm -f "$dest/vocab.json"; continue;
  }
  if curl -L --fail -o "$dest/model.onnx" "$base/onnx/$ONNX_FILE"; then
    echo "[w2v] $lang model.onnx fetched ($(du -h "$dest/model.onnx" | cut -f1))"
  else
    echo "[w2v] $lang model.onnx fetch failed from $base/onnx/$ONNX_FILE"
    rm -f "$dest/model.onnx"
  fi
done

echo ""
echo "Done."
