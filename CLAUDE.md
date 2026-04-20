# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Frontend dev server (port 1420, HMR 1421)
pnpm dev

# Full Tauri dev (frontend + Rust backend hot-reload)
pnpm tauri dev

# Build
pnpm build                        # frontend only
pnpm tauri build                  # full desktop app
pnpm tauri build --features metal # Apple Silicon: Metal GPU for Whisper (recommended)

# Download required sidecar binaries (yt-dlp, demucs, ggml-tiny.bin ~77 MB)
./scripts/fetch-binaries.sh

# Alignment regression tests (downloads Whisper Small ~500 MB on first run)
cargo test -p aligner-whisper --test regression -- --nocapture
# With Metal GPU (Whisper Medium):
cargo test -p aligner-whisper --test regression --features metal -- --nocapture

# Regenerate TTS test fixtures (macOS only, requires say -v Alice)
python3 scripts/gen-test-fixtures.py
```

No lint scripts are configured.

## Architecture

Desktop karaoke app. Frontend is React 18 + Vite. Backend is Tauri 2 + Rust. No routing — view switching is plain `useState` in `App.jsx`.

### Frontend → Backend communication

- **Commands**: `invoke("command_name", { args })` — request/response
- **Events**: `listen("karaoke://jobs", handler)` — backend pushes progress/state updates

### Core data flow

1. User submits YouTube URL → `jobs_enqueue` Tauri command
2. `jobs.rs` spawns async worker, runs `pipeline.rs` (7 stages)
3. Each stage emits progress via `karaoke://jobs` event
4. `jobsContext.jsx` (global React Context) receives events, updates UI
5. On completion, `App.jsx` calls `scan_library` → library refreshes

### Pipeline stages (pipeline.rs)

0. Download audio (yt-dlp sidecar)
1. Fetch lyrics (lrclib.net)
2. Fetch album art (iTunes API → Cover Art Archive fallback)
3. Separate vocals (demucs sidecar)
4. Align words — pure-Rust `ForcedAligner` (Whisper cross-attention + DTW); falls back to whisper-apr sidecar if unavailable
5. Compute reference pitch (YIN algorithm)
6. Save metadata.json

### Rust modules (src-tauri/src/)

| Module | Role |
|---|---|
| `jobs.rs` | Async job queue; emits `karaoke://jobs` and `karaoke://jobs-list` events |
| `pipeline.rs` | Orchestrates all 7 pipeline stages; progress callbacks |
| `library.rs` | Scan library dir, read/write metadata.json per song |
| `downloader.rs` | yt-dlp wrapper |
| `separator.rs` | demucs sidecar invocation |
| `aligner.rs` | Word alignment: tries pure-Rust `ForcedAligner` first, falls back to whisper-apr sidecar |
| `pitch.rs` | YIN pitch detector; precompute reference contour + real-time Challenge scoring |
| `recorder.rs` | Mic capture via cpal; writes WAV during Challenge play |
| `leaderboard.rs` | SQLite (bundled via rusqlite); per-song + global top scores |
| `recognizer.rs` | Shazam-style fingerprinting in pure Rust |
| `settings.rs` | Persistent settings (YouTube cookie bypass config) |

### Rust workspace crates (crates/)

| Crate | Role |
|---|---|
| `aligner-pipeline` | Shared types: `AudioBuffer`, `AlignedWord`, `TimelineEntry`, `Progress` |
| `aligner-whisper` | Forced word-level alignment via Whisper cross-attention + DTW. Pure Rust (candle). No subprocess. |

#### aligner-whisper internals

- `model.rs` — Whisper encoder + decoder loaded from HuggingFace safetensors (hf-hub). Weights cached at `~/.cache/huggingface/hub/`. Uses `candle-core/nn` 0.8. Key: `ForcedAlignDecoder::forced_attention()` runs full teacher-forced sequence with causal mask in a single pass to get valid cross-attention.
- `mel.rs` — Log-mel spectrogram (N_FFT=400, HOP=160, 80 mels). Silence detection via mel energy for DTW truncation.
- `dtw.rs` — O(n×m) dynamic time warping + traceback.
- `normalize.rs` — Italian/English contraction expansion + unicode strip.
- `lib.rs` — `ForcedAligner`: chunked alignment, median filter on attention, silence-truncated DTW, 0.65× span back-shift (compensates Whisper attention lagging word onset).

**Model selection**: `WhisperModel::Small` on CPU (default), `WhisperModel::Medium` with `--features metal`. MAE < 150ms, P90 < 300ms on Italian TTS fixture with Small.

**Bias gotcha**: Whisper's q_proj/v_proj/out_proj/fc1/fc2 all have biases; k_proj does not. Use `linear()` not `linear_no_bias()` for those layers or attention is garbage.

### Frontend views

- **Library.jsx** — song cards; play/delete
- **Download.jsx** — URL input; live pipeline stage visualization per job
- **Player.jsx** — lyrics scroll, waveform, instrumental/original toggle, Challenge mode (mic + real-time pitch), leaderboard
- **Leaderboard.jsx** — per-song top 10 + global top; podium for top 3
- **Settings.jsx** — YouTube cookies/browser preferences

### Key frontend patterns

- `jobsContext.jsx` — only global state; manages background job list via Tauri events
- `App.jsx` — root; owns current view + current song; triggers library refresh on job completion
- CSS: token-based via `tokens.css` CSS variables; no UI library; brand gradient is `CK_GRADIENT = "linear-gradient(135deg, #FFB370 0%, #FF6B5A 40%, #F23D6D 100%)"`
- `Background.jsx` — Aurora animation in Player, Threads elsewhere; respects `prefers-reduced-motion`
- Real-time events used in: `jobsContext` (`karaoke://jobs`, `karaoke://jobs-list`), `Player` (`karaoke://score-tick`)
