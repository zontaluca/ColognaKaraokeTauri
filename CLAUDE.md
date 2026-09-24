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
pnpm tauri build --features metal # Apple Silicon: CoreML execution provider for wav2vec2 ONNX

# Download required sidecar binaries (yt-dlp, demucs) + wav2vec2 ONNX models
./scripts/fetch-binaries.sh
```

No lint scripts are configured. Rust unit tests: `cargo test --workspace` (the aligner crate links ONNX Runtime, which `ort` downloads at build time).

Cargo profiles (root `Cargo.toml`): release uses thin LTO + `codegen-units = 1`; dev builds compile dependencies with `opt-level = 2` so the audio pipeline is usable under `pnpm tauri dev` (slower first build).

## Architecture

Desktop karaoke app. Frontend is React 18 + Vite. Backend is Tauri 2 + Rust. No routing — view switching is plain `useState` in `App.jsx`.

### Frontend → Backend communication

- **Commands**: `invoke("command_name", { args })` — request/response
- **Events**: `listen("karaoke://jobs", handler)` — backend pushes progress/state updates

### Core data flow

1. User submits YouTube URL → `jobs_enqueue` Tauri command
2. `jobs.rs` spawns async worker, runs `pipeline.rs` (7 stages)
3. Each stage emits progress via `karaoke://jobs` event (rate-limited: stage/status/message-kind changes always pass, percentage ticks at most every 200 ms; `done` is emitted once, when the pipeline returns)
4. `jobsContext.jsx` (global React Context) receives events, updates UI
5. On completion, `App.jsx` calls `scan_library` → library refreshes

CPU-heavy work (decoding, resampling, ONNX inference, CTC, MP3 encoding, pitch contour) runs in `tokio::task::spawn_blocking`, never directly on the async runtime. Sync Tauri commands that touch disk or devices are declared `#[tauri::command(async)]` so they don't run on the main (UI) thread.

### Pipeline stages (pipeline.rs)

0. Download audio (yt-dlp sidecar)
1. Recognize song (Shazam fingerprint of the middle 12 s, best-effort) + fetch lyrics (lrclib.net)
2. Fetch album art (iTunes API → Cover Art Archive fallback)
3. Separate vocals (demucs sidecar); stems are mixed/encoded to `instrumental.mp3` + `vocals.mp3` in parallel
4. Align words — wav2vec2 CTC forced alignment (`aligner-wav2vec2`, ONNX Runtime)
5. Compute reference pitch (YIN) → `pitch.json`, on its own thread concurrently with step 4
6. Save metadata.json

Steps 4 and 5 share a single decode of `vocals.mp3` (`pipeline::align_and_pitch`, also used by `reprocess_song`). The loaded wav2vec2 session is cached between queued jobs and released when the queue drains (`aligner::release_model_cache`).

Per-song files: `metadata.json`, `original.mp3`, `instrumental.mp3`, `vocals.mp3`, `words.json`, `pitch.json`, `cover.jpg`, `recordings/`. Cloud sync (`cloud.rs` `SYNC_FILES`) must list every artifact needed to use a song after restore.

### Rust modules (src-tauri/src/)

| Module | Role |
|---|---|
| `jobs.rs` | Async job queue; emits `karaoke://jobs` and `karaoke://jobs-list` events |
| `pipeline.rs` | Orchestrates all 7 pipeline stages; progress callbacks |
| `library.rs` | Scan library dir, read/write metadata.json per song |
| `audio.rs` | Symphonia decode to mono f32 (full track or middle excerpt) + rubato resampling |
| `http.rs` | Shared `reqwest::Client` + `urlencode` (use it instead of building clients) |
| `downloader.rs` | yt-dlp wrapper |
| `separator.rs` | demucs sidecar invocation; streaming stem mix + chunked LAME encoding |
| `aligner.rs` | Word alignment via `aligner-wav2vec2`; maps aligned words back to LRC lines (`line` field) |
| `lyrics.rs` | lrclib.net fetch + LRC parsing (strips enhanced `<mm:ss.xx>` stamps like the frontend) |
| `metadata.rs` | Album metadata + cover download |
| `pitch.rs` | YIN pitch detector; precompute reference contour + real-time Challenge scoring (dedicated thread) |
| `recorder.rs` | Mic capture via cpal; writes WAV during Challenge play |
| `leaderboard.rs` | SQLite (bundled via rusqlite); per-song + global top scores |
| `recognizer.rs` | Shazam-style fingerprinting in pure Rust |
| `cloud.rs` | MEGA sync through the MEGAcmd CLI |
| `players.rs` | Party queue persistence |
| `settings.rs` | Persistent settings (YouTube cookie bypass config) |

### Rust workspace crates (crates/)

| Crate | Role |
|---|---|
| `aligner-pipeline` | Shared types: `AudioBuffer`, `AlignedWord`, `TimelineEntry`, `Progress`; `detect_vocal_range` |
| `aligner-wav2vec2` | Forced word-level alignment via wav2vec2 CTC (ONNX Runtime through `ort`). No subprocess. |

#### aligner-wav2vec2 internals

- `model.rs` — ONNX session (`model.onnx` + `vocab.json` from `default_local_dir(lang)`, i.e. `<cache>/cologna-karaoke/wav2vec2/<lang>/`, fetched/exported by `scripts/fetch-binaries.sh` / `scripts/export-wav2vec2-onnx.py`). Execution provider pinned explicitly (CPU, or CoreML/CUDA via features). `log_softmax_rows`.
- `audio.rs` — 80 Hz high-pass biquad + RMS normalisation before inference.
- `text.rs` — lyrics → char-level CTC targets with `|` delimiters. Tokens with no letters are dropped: use `count_alignable_words` when mapping aligned words back to lines.
- `ctc.rs` — Viterbi forced alignment over the blank-extended target (two rolling alpha rows + u8 backpointers).
- `lib.rs` — `Wav2vecAligner`: 20 s non-overlapping inference chunks, CTC, char spans → word spans; `align_owned` avoids copying the input.

**Language**: picked per song by stopword counting on the LRC (`detect_lrc_language`, default `it`); the matching model directory must exist or alignment writes an empty `words.json`.

### Frontend views

- **Library.jsx** — song cards; play/delete
- **Download.jsx** — URL input; live pipeline stage visualization per job
- **Player.jsx** — lyrics scroll, waveform, instrumental/original toggle, Challenge mode (mic + real-time pitch), leaderboard
- **Leaderboard.jsx** — per-song top 10 + global top; podium for top 3
- **Settings.jsx** — YouTube cookies/browser preferences

### Key frontend patterns

- `jobsContext.jsx` — only global state; manages background job list via Tauri events
- `hooks/useTauriEvent.js` — subscribe to a Tauri event from a component (handler via ref, no leaked listeners if unmounted before `listen()` resolves). Prefer it over hand-written `listen` effects
- `main.jsx` lazy-loads `App` or `PresentationView`, so each window only loads its own bundle
- Player: lyric lines (`LyricLine`) and the waveform are memoized; playback position is computed by `computePosition` (shared by the rAF loop and seeking). Challenge score ticks carry `word_idx` = index in `words.json`; words in `wordsByLine` carry it as `gi`
- `App.jsx` — root; owns current view + current song; triggers library refresh on job completion
- CSS: token-based via `tokens.css` CSS variables; no UI library; brand gradient is `CK_GRADIENT = "linear-gradient(135deg, #FFB370 0%, #FF6B5A 40%, #F23D6D 100%)"`
- `Background.jsx` — Aurora animation in Player (rendered at 1/4 resolution), Threads elsewhere; capped at 30 fps, paused while hidden; respects `prefers-reduced-motion`
- Real-time events used in: `jobsContext` (`karaoke://jobs`, `karaoke://jobs-list`), `Player` (`karaoke://score-tick`)
