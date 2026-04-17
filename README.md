# Cologna Karaoke — Tauri 2 + React

Desktop karaoke with Spotify-style lyrics, background job queue, SingStar-style
realtime pitch scoring, and a local SQLite leaderboard. Paste a YouTube URL →
downloads audio, fetches synced LRC lyrics + album art, separates vocals,
aligns words via `whisper-rs`, and caches a reference pitch contour.

All processing is **local** and **offline-capable** after first run. No Python
dependencies — transcription runs in pure Rust via `whisper-rs` (whisper.cpp).

## Stack

- **Frontend:** React 18 + Vite, plain CSS with token-based design system
- **Shell:** Tauri 2 (Rust)
- **Sidecar binaries:** `yt-dlp` (audio download), `demucs` ([demucs-rs](https://github.com/nikhilunni/demucs-rs) native inference)
- **Rust crates:** `whisper-rs` (word alignment), `cpal` (mic I/O), `pitch-detection` (YIN),
  `rusqlite` (leaderboard), `rubato` (resampling), `symphonia` (audio decode), `hound` (WAV)
- **Lyrics:** [lrclib.net](https://lrclib.net)
- **Album art:** iTunes Search API, Cover Art Archive fallback

## Prerequisites

- Node 20+ & pnpm
- Rust stable toolchain
- `cmake` (required by `whisper-rs-sys` build script — `brew install cmake` on macOS)
- `ffmpeg` in `PATH` (yt-dlp mp3 extraction)

## Setup

```bash
pnpm install
./scripts/fetch-binaries.sh    # yt-dlp + demucs + ggml-tiny.bin (~77 MB)
pnpm tauri dev
```

First demucs run downloads htdemucs weights (~84 MB) to OS cache.
Mic permission is requested on first Challenge-mode play (macOS prompts via `NSMicrophoneUsageDescription`).

## Features

- **Spotify-style lyrics**: full-viewport scroll, past/current/future color states, current word highlight.
- **Background jobs**: queue many URLs, navigate while processing. Toast shows active job.
- **Challenge mode**: records mic via `cpal`, runs YIN pitch detection live, scores per word (hit/partial/miss), persists top scores to SQLite.
- **Classifica view**: per-song top 10 + global top.
- **Animated backgrounds**: Aurora-style drift in Player, Threads in other views. Respects `prefers-reduced-motion`.
- **Album art**: fetched during processing, displayed on cards + Player.

## Pipeline stages

0. Download audio (`yt-dlp`)
1. Fetch LRC lyrics (`lrclib`)
2. Fetch album art (`iTunes` / `Cover Art Archive`)
3. Separate vocals (`demucs-rs`)
4. Align words (`whisper-rs` with LRC as initial prompt) — **mandatory**
5. Compute reference pitch contour (`pitch-detection` YIN)
6. Save metadata

Progress streams via `karaoke://jobs` events (and legacy `karaoke://progress`).

## Rust commands

| Command | Description |
|---|---|
| `scan_library` / `delete_song` / `get_library_dir` / `get_song_audio_path` | Library basics |
| `fetch_lyrics_cmd` / `parse_lrc_cmd` | Lyrics |
| `process_youtube_url` | Legacy sync path |
| `jobs_enqueue` / `jobs_list` / `jobs_cancel` | Background queue |
| `get_words` | Read cached word alignments |
| `recorder_start` / `recorder_stop` | Mic WAV capture (Challenge only) |
| `pitch_start` / `pitch_stop` | Realtime score-tick engine |
| `leaderboard_insert` / `leaderboard_top` / `leaderboard_global_top` | SQLite leaderboard |

Events: `karaoke://jobs`, `karaoke://jobs-list`, `karaoke://score-tick`.

## Layout

```
/src             React app
  /views         Library / Download / Player / Leaderboard
  /components    Sidebar, Background
  jobsContext.jsx global jobs store + toast
  /styles        tokens.css, global.css
/src-tauri       Rust backend
  /src
    library.rs downloader.rs separator.rs lyrics.rs
    pipeline.rs aligner.rs audio.rs metadata.rs
    jobs.rs recorder.rs pitch.rs leaderboard.rs
  /binaries      sidecar binaries (not committed)
  /resources/models/ggml-tiny.bin   whisper model (not committed; fetch via script)
  Info.plist     macOS NSMicrophoneUsageDescription
  entitlements.plist  com.apple.security.device.microphone
```

## Python dependencies

None. The old Python-based alignment (`stable-ts`, `openai-whisper`) was replaced
by pure Rust `whisper-rs`. `yt-dlp` remains as a pre-built single binary and does
not require a Python interpreter.
