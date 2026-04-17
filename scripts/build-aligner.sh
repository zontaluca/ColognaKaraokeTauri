#!/usr/bin/env bash
# Build the 'aligner' sidecar: PyInstaller bundles stable_whisper + openai-whisper
# into a single self-contained binary placed in src-tauri/binaries/ with the
# Tauri target-triple suffix required for sidecar lookup.
#
# Run once after cloning (and whenever you update stable-ts / whisper versions).
# Output size is large (~500 MB) because it embeds PyTorch. This is unavoidable
# while using whisper in Python.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$HERE/.."
SRC="$HERE/aligner/aligner.py"
BIN_DIR="$ROOT/src-tauri/binaries"
VENV="$HERE/aligner/.venv"

mkdir -p "$BIN_DIR"

TRIPLE="$(rustc -vV | sed -n 's|host: ||p')"
OUT_NAME="aligner-$TRIPLE"
case "$TRIPLE" in
  *windows*)
    OUT_NAME="aligner-$TRIPLE.exe"
    ;;
esac
OUT_PATH="$BIN_DIR/$OUT_NAME"

echo "Target: $TRIPLE"
echo "Output: $OUT_PATH"

# Ensure python3 is available
if ! command -v python3 >/dev/null 2>&1; then
  echo "python3 not found on PATH. Install Python 3.11+ and retry." >&2
  exit 1
fi

# Build venv with pinned deps
if [[ ! -d "$VENV" ]]; then
  python3 -m venv "$VENV"
fi
# shellcheck disable=SC1091
source "$VENV/bin/activate"
python -m pip install --upgrade pip wheel setuptools
python -m pip install \
  "stable-ts>=2.17" \
  "openai-whisper>=20231117" \
  "torch" \
  "pyinstaller>=6.3"

DIST="$HERE/aligner/dist"
BUILD="$HERE/aligner/build"
SPEC="$HERE/aligner/aligner.spec"
rm -rf "$DIST" "$BUILD" "$SPEC"

pyinstaller \
  --name aligner \
  --onefile \
  --noconfirm \
  --clean \
  --collect-all whisper \
  --collect-all stable_whisper \
  --copy-metadata openai-whisper \
  --copy-metadata stable-ts \
  --collect-data tiktoken_ext \
  --hidden-import tiktoken \
  --hidden-import tiktoken_ext \
  --hidden-import tiktoken_ext.openai_public \
  --distpath "$DIST" \
  --workpath "$BUILD" \
  --specpath "$HERE/aligner" \
  "$SRC"

EXE="$DIST/aligner"
[[ -f "$EXE.exe" ]] && EXE="$EXE.exe"
if [[ ! -f "$EXE" ]]; then
  echo "PyInstaller output not found at $EXE" >&2
  exit 1
fi

mv "$EXE" "$OUT_PATH"
chmod +x "$OUT_PATH"

echo "Done. Built $OUT_PATH"
ls -la "$BIN_DIR"
