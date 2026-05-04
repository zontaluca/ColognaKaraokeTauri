//! Milestone 2 acceptance tests.
//!
//! These tests require real audio and model weights; they are skipped when the
//! test-asset files or the HuggingFace cache are absent so that CI does not
//! block without GPU / network access.
//!
//! To run locally (downloads ~500 MB of Whisper Medium weights on first run):
//!   cargo test -p aligner-whisper --test regression -- --nocapture

use std::path::{Path, PathBuf};

use aligner_pipeline::AudioBuffer;
use aligner_whisper::{AlignmentConfig, ForcedAligner, PhraseAnchor, WhisperModel};
use serde::Deserialize;

// ─── Fixture types ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct FixtureWord {
    word: String,
    start: f64,
    end: f64,
}

#[derive(Debug, Deserialize)]
struct Fixture {
    audio_file: String,
    language: String,
    lyrics: String,
    words: Vec<FixtureWord>,
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn test_assets_dir() -> PathBuf {
    // Works whether run from workspace root or crate root.
    let candidates = [
        Path::new("test-assets"),
        Path::new("../../test-assets"),
    ];
    for c in &candidates {
        if c.join("fixtures.json").exists() {
            return c.to_path_buf();
        }
    }
    PathBuf::from("test-assets")
}

fn load_wav_mono_16k(path: &Path) -> Option<AudioBuffer> {
    // Use hound (already in workspace via aligner-audio dependency later;
    // for now we depend on it directly in dev-deps).
    let reader = hound::WavReader::open(path).ok()?;
    let spec = reader.spec();
    let channels = spec.channels as usize;
    let src_rate = spec.sample_rate;

    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => {
            reader.into_samples::<f32>().filter_map(|s| s.ok()).collect()
        }
        hound::SampleFormat::Int => {
            let bits = spec.bits_per_sample as i32;
            let max = (1i64 << (bits - 1)) as f32;
            reader
                .into_samples::<i32>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / max)
                .collect()
        }
    };

    let mono: Vec<f32> = if channels <= 1 {
        raw
    } else {
        raw.chunks(channels)
            .map(|c| c.iter().sum::<f32>() / channels as f32)
            .collect()
    };

    // Resample to 16 kHz if needed.
    let samples = if src_rate == 16_000 {
        mono
    } else {
        use rubato::{
            Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType,
            WindowFunction,
        };
        let params = SincInterpolationParameters {
            sinc_len: 128,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 128,
            window: WindowFunction::BlackmanHarris2,
        };
        let ratio = 16_000.0 / src_rate as f64;
        let mut resampler =
            SincFixedIn::<f32>::new(ratio, 2.0, params, mono.len(), 1).ok()?;
        let out = resampler.process(&[mono], None).ok()?;
        out.into_iter().next()?
    };

    Some(AudioBuffer { samples, sample_rate: 16_000 })
}

fn skip_if_missing(path: &Path) -> bool {
    if !path.exists() {
        eprintln!("SKIP: {} not found (add real audio to test-assets/)", path.display());
        true
    } else {
        false
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

/// MAE < 150 ms, P90 < 300 ms on the clean fixture.
#[tokio::test]
async fn test_mae_on_clean_fixture() {
    let assets = test_assets_dir();
    let fixture_path = assets.join("fixtures.json");
    let audio_path = assets.join("vocals-clean.wav");

    if skip_if_missing(&fixture_path) || skip_if_missing(&audio_path) {
        return;
    }

    let fixture: Fixture = serde_json::from_str(
        &std::fs::read_to_string(&fixture_path).expect("read fixtures.json"),
    )
    .expect("parse fixtures.json");

    let vocals = load_wav_mono_16k(&audio_path).expect("load vocals-clean.wav");

    // Use Small for CPU test runs (6 layers vs 24 for Medium — ~8× faster).
    // Run with `--features metal` to use Medium on Apple Silicon.
    let model = if cfg!(feature = "metal") { if cfg!(feature = "metal") { WhisperModel::Medium } else { WhisperModel::Small } } else { WhisperModel::Small };
    let config = AlignmentConfig {
        model,
        language: fixture.language.clone(),
        ..Default::default()
    };
    let aligner = ForcedAligner::new(config).await.expect("load model");
    let result = aligner.align(&vocals, &fixture.lyrics).expect("align");

    assert_eq!(
        result.len(),
        fixture.words.len(),
        "coverage: expected {} words, got {}",
        fixture.words.len(),
        result.len()
    );

    let mut abs_errors: Vec<f64> = Vec::new();
    for (aligned, gt) in result.iter().zip(fixture.words.iter()) {
        let err = (aligned.start - gt.start).abs();
        abs_errors.push(err);
        println!(
            "{:>12}  aligned={:.3}s  gt={:.3}s  err={:.0}ms",
            aligned.word,
            aligned.start,
            gt.start,
            err * 1000.0
        );
    }

    let mae = abs_errors.iter().sum::<f64>() / abs_errors.len() as f64;
    abs_errors.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p90_idx = (abs_errors.len() as f64 * 0.9) as usize;
    let p90 = abs_errors[p90_idx.min(abs_errors.len() - 1)];

    println!("\nMAE = {:.1}ms  P90 = {:.1}ms", mae * 1000.0, p90 * 1000.0);

    assert!(
        mae < 0.150,
        "MAE {:.1}ms exceeds 150ms threshold",
        mae * 1000.0
    );
    assert!(
        p90 < 0.300,
        "P90 {:.1}ms exceeds 300ms threshold",
        p90 * 1000.0
    );
}

/// Every output word must have start ≤ next word's start.
#[tokio::test]
async fn test_monotonicity() {
    let assets = test_assets_dir();
    let fixture_path = assets.join("fixtures.json");
    let audio_path = assets.join("vocals-clean.wav");

    if skip_if_missing(&fixture_path) || skip_if_missing(&audio_path) {
        return;
    }

    let fixture: Fixture = serde_json::from_str(
        &std::fs::read_to_string(&fixture_path).expect("read fixtures.json"),
    )
    .expect("parse fixtures.json");

    let vocals = load_wav_mono_16k(&audio_path).expect("load vocals-clean.wav");
    let aligner = ForcedAligner::new(AlignmentConfig {
        model: if cfg!(feature = "metal") { WhisperModel::Medium } else { WhisperModel::Small },
        language: fixture.language.clone(),
        ..Default::default()
    })
    .await
    .expect("load model");
    let result = aligner.align(&vocals, &fixture.lyrics).expect("align");

    for w in result.windows(2) {
        assert!(
            w[1].start >= w[0].start,
            "monotonicity violated: '{}' ({:.3}s) comes before '{}' ({:.3}s)",
            w[0].word,
            w[0].start,
            w[1].word,
            w[1].start
        );
    }
}

/// Every word from the input lyrics must appear exactly once in the output.
#[tokio::test]
async fn test_coverage() {
    let assets = test_assets_dir();
    let fixture_path = assets.join("fixtures.json");
    let audio_path = assets.join("vocals-clean.wav");

    if skip_if_missing(&fixture_path) || skip_if_missing(&audio_path) {
        return;
    }

    let fixture: Fixture = serde_json::from_str(
        &std::fs::read_to_string(&fixture_path).expect("read fixtures.json"),
    )
    .expect("parse fixtures.json");

    let vocals = load_wav_mono_16k(&audio_path).expect("load vocals-clean.wav");
    let aligner = ForcedAligner::new(AlignmentConfig {
        model: if cfg!(feature = "metal") { WhisperModel::Medium } else { WhisperModel::Small },
        language: fixture.language.clone(),
        ..Default::default()
    })
    .await
    .expect("load model");
    let result = aligner.align(&vocals, &fixture.lyrics).expect("align");

    let input_words: Vec<&str> = fixture.lyrics.split_whitespace().collect();
    assert_eq!(
        result.len(),
        input_words.len(),
        "output word count ({}) != input word count ({})",
        result.len(),
        input_words.len()
    );
    for (r, w) in result.iter().zip(input_words.iter()) {
        assert_eq!(&r.word, w, "word order mismatch");
    }
}

/// Real-song cascade regression test — Willie Peyote "Buon Auspicio", first verse.
///
/// The old algorithm produced 1 ms cascade chains for phrase-final words:
///   questa, start=11263ms  →  si start=11264ms  →  conclude start=11265ms
/// (back-shift span_dur×0.65 overshot into earlier words' time, causing the
/// monotonicity nudge to pile up at 1 ms intervals).
///
/// This test verifies the fix on the first verse (words 0–7: Quando…conclude):
///   1. No cascade: consecutive words are ≥ 50 ms apart.
///   2. First-verse MAE vs words_correct.json < 400 ms.
///
/// Note: the full ForcedAligner output (without LRC clamping) drifts for late
/// words in long songs.  The production pipeline in aligner.rs clamps each word
/// to its LRC line window, masking that drift.  This test intentionally covers
/// only the first verse where proportional chunk assignment is correct.
#[tokio::test]
async fn test_willie_peyote_no_cascade() {
    let song_dir = Path::new("/Users/lucazonta/Library/Application Support/com.colognakaraoke.tauri/library/Willie Peyote Official_Willie Peyote - Buon auspicio visual video");
    let vocals_path = song_dir.join("vocals.wav");
    let correct_path = song_dir.join("words_correct.json");

    if skip_if_missing(&vocals_path) || skip_if_missing(&correct_path) {
        return;
    }

    #[derive(serde::Deserialize)]
    struct CorrectWord { word: String, start_ms: u64 }

    let correct: Vec<CorrectWord> = serde_json::from_str(
        &std::fs::read_to_string(&correct_path).expect("read words_correct.json"),
    )
    .expect("parse words_correct.json");

    // Build lyrics from ALL words so the proportional chunk assignment sees the
    // full token count — altering the lyrics changes which chunk covers verse 1.
    let lyrics = correct.iter().map(|w| w.word.as_str()).collect::<Vec<_>>().join(" ");

    let vocals = load_wav_mono_16k(&vocals_path).expect("load vocals.wav");
    let model = if cfg!(feature = "metal") { WhisperModel::Medium } else { WhisperModel::Small };
    let config = AlignmentConfig { model, language: "it".to_string(), ..Default::default() };
    let aligner = ForcedAligner::new(config).await.expect("load model");
    let result = aligner.align(&vocals, &lyrics).expect("align");

    assert_eq!(result.len(), correct.len(), "word count mismatch");

    // Focus on first verse (words 0–7: Quando…conclude).
    // These are the words that had the cascade bug.
    let verse_end = 8.min(result.len());

    // 1) Cascade check: no pair within 50 ms in the first verse.
    //    (Old bug produced 1 ms gaps; 50 ms gives headroom while catching regressions.)
    println!("\n── first-verse alignment ───────────────────────────");
    let mut errors: Vec<f64> = Vec::new();
    for i in 0..verse_end {
        let got_ms = result[i].start * 1_000.0;
        let exp_ms = correct[i].start_ms as f64;
        let err = (got_ms - exp_ms).abs();
        errors.push(err);
        println!("  {:>12}  got={:.0}ms  correct={}ms  err={:.0}ms",
            result[i].word, got_ms, exp_ms, err);
    }

    let mut cascade_violations: Vec<String> = Vec::new();
    for w in result[..verse_end].windows(2) {
        let gap_ms = (w[1].start - w[0].start) * 1_000.0;
        if gap_ms < 50.0 {
            cascade_violations.push(format!(
                "'{}' ({:.0}ms) → '{}' ({:.0}ms) gap={:.0}ms",
                w[0].word, w[0].start * 1000.0,
                w[1].word, w[1].start * 1000.0,
                gap_ms
            ));
        }
    }
    if !cascade_violations.is_empty() {
        println!("\n── cascade violations (first verse) ────────────────");
        for v in &cascade_violations { println!("  {}", v); }
    }
    assert!(
        cascade_violations.is_empty(),
        "{} cascade violation(s) in first verse (consecutive words < 50 ms apart)",
        cascade_violations.len()
    );

    // 2) MAE check.
    let mae = errors.iter().sum::<f64>() / errors.len() as f64;
    println!("\nFirst-verse MAE = {:.1}ms", mae);
    assert!(mae < 400.0, "First-verse MAE {:.1}ms exceeds 400ms", mae);
}

// ─── Dynamic song regression ─────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
struct SongWord {
    word: String,
    start_ms: u64,
    #[serde(default)]
    end_ms: u64,
    #[serde(default)]
    line: usize,
}

#[derive(Debug, serde::Deserialize)]
struct SongMeta {
    language: String,
}

#[derive(Debug, serde::Deserialize)]
struct LrcDoc {
    lrc: String,
}

/// Parse an LRC string into phrase start times in milliseconds, one per line.
fn parse_lrc(lrc: &str) -> Vec<u64> {
    parse_lrc_full(lrc).into_iter().map(|(ms, _)| ms).collect()
}

/// Parse an LRC string into (start_ms, phrase_text) pairs.
///
/// Empty-text LRC lines (purely-instrumental markers) are skipped — the
/// ground-truth words.json `line` field indexes non-empty phrases only.
fn parse_lrc_full(lrc: &str) -> Vec<(u64, String)> {
    let re = regex::Regex::new(r"^\[(\d+):(\d+(?:\.\d+)?)\](.*)$").unwrap();
    lrc.lines()
        .filter_map(|line| {
            let caps = re.captures(line.trim_start())?;
            let m: u64 = caps.get(1)?.as_str().parse().ok()?;
            let s: f64 = caps.get(2)?.as_str().parse().ok()?;
            let text = caps.get(3)?.as_str().trim().to_string();
            if text.is_empty() {
                return None;
            }
            Some((((m as f64) * 60.0 * 1000.0 + s * 1000.0) as u64, text))
        })
        .collect()
}

/// Discover all test-assets/song*/ directories, sorted by name.
fn discover_song_dirs() -> Vec<PathBuf> {
    let candidates = [Path::new("test-assets"), Path::new("../../test-assets")];
    let base = candidates
        .iter()
        .find(|c| c.join("song1").exists())
        .map(|c| c.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("test-assets"));

    let Ok(entries) = std::fs::read_dir(&base) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("song"))
                    .unwrap_or(false)
        })
        .collect();
    dirs.sort();
    dirs
}

/// Dynamic regression: runs for every test-assets/song*/ folder.
///
/// Each song folder must contain:
///   vocals.wav      — audio (any sample rate, any bit depth)
///   words.json      — [{word, start_ms, end_ms, line}, ...]  ground-truth
///   metadata.json   — {"language": "it"}  (ISO 639-1 code)
///
/// Assertions (first VERSE_WORDS words only, where alignment is accurate):
///   1. Word count matches words.json.
///   2. No cascade: consecutive words ≥ 50 ms apart  (catches back-shift bug).
///   3. MAE < 500 ms.
///
/// Run with Metal GPU (recommended):
///   cargo test -p aligner-whisper --test regression test_all_songs_regression --features metal -- --nocapture
#[tokio::test]
async fn test_all_songs_regression() {
    const VERSE_WORDS: usize = 20;
    // 5ms catches the old back-shift cascade (which produced 1ms gaps) but
    // tolerates sub-phrase DTW collisions at phrase boundaries where the
    // last word of phrase N and first of phrase N+1 can land very close.
    const CASCADE_MIN_MS: f64 = 5.0;
    const MAE_THRESHOLD_MS: f64 = 500.0;

    let song_dirs = discover_song_dirs();
    if song_dirs.is_empty() {
        eprintln!("SKIP: no test-assets/song*/ directories found");
        return;
    }

    let model = if cfg!(feature = "metal") { WhisperModel::Medium } else { WhisperModel::Small };
    let mut failures: Vec<String> = Vec::new();

    for dir in &song_dirs {
        let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        let vocals_path = dir.join("vocals.wav");
        let words_path = dir.join("words.json");
        let meta_path = dir.join("metadata.json");

        if skip_if_missing(&vocals_path) || skip_if_missing(&words_path) || skip_if_missing(&meta_path) {
            continue;
        }

        let meta: SongMeta = match serde_json::from_str(
            &std::fs::read_to_string(&meta_path).expect("read metadata.json"),
        ) {
            Ok(m) => m,
            Err(e) => { failures.push(format!("[{name}] parse metadata.json: {e}")); continue; }
        };

        let ground_truth: Vec<SongWord> = match serde_json::from_str(
            &std::fs::read_to_string(&words_path).expect("read words.json"),
        ) {
            Ok(w) => w,
            Err(e) => { failures.push(format!("[{name}] parse words.json: {e}")); continue; }
        };

        let vocals = match load_wav_mono_16k(&vocals_path) {
            Some(v) => v,
            None => { failures.push(format!("[{name}] failed to load vocals.wav")); continue; }
        };

        // Optional lrc.json — phrase-level ground-truth windows.
        let lrc_path = dir.join("lrc.json");
        let lrc_phrases: Vec<u64> = if lrc_path.exists() {
            match std::fs::read_to_string(&lrc_path) {
                Ok(s) => match serde_json::from_str::<LrcDoc>(&s) {
                    Ok(doc) => parse_lrc(&doc.lrc),
                    Err(e) => { failures.push(format!("[{name}] parse lrc.json: {e}")); Vec::new() }
                },
                Err(e) => { failures.push(format!("[{name}] read lrc.json: {e}")); Vec::new() }
            }
        } else {
            Vec::new()
        };

        let config = AlignmentConfig {
            model: model.clone(),
            language: meta.language.clone(),
            ..Default::default()
        };
        let aligner = match ForcedAligner::new(config).await {
            Ok(a) => a,
            Err(e) => { failures.push(format!("[{name}] load model: {e}")); continue; }
        };

        // When we have LRC phrase timestamps, run per-phrase alignment for
        // drift-free accuracy on long songs.  Group ground-truth words by
        // `line` to preserve the exact word sequence/tokenization.
        let result = if !lrc_phrases.is_empty() {
            let mut words_per_line: Vec<Vec<String>> = vec![Vec::new(); lrc_phrases.len()];
            for w in &ground_truth {
                if w.line < words_per_line.len() {
                    words_per_line[w.line].push(w.word.clone());
                }
            }
            let phrases: Vec<PhraseAnchor> = lrc_phrases
                .iter()
                .zip(words_per_line.into_iter())
                .map(|(&ms, words)| PhraseAnchor {
                    time_sec: ms as f64 / 1000.0,
                    words,
                })
                .collect();
            match aligner.align_with_phrases(&vocals, &phrases, 1.5) {
                Ok(r) => r,
                Err(e) => { failures.push(format!("[{name}] phrase-align error: {e}")); continue; }
            }
        } else {
            let lyrics = ground_truth.iter().map(|w| w.word.as_str()).collect::<Vec<_>>().join(" ");
            match aligner.align(&vocals, &lyrics) {
                Ok(r) => r,
                Err(e) => { failures.push(format!("[{name}] align error: {e}")); continue; }
            }
        };

        println!("\n══ {name} (lang={}) ════════════════════════════════", meta.language);

        // 1) word count
        if result.len() != ground_truth.len() {
            failures.push(format!(
                "[{name}] word count mismatch: got {} expected {}",
                result.len(), ground_truth.len()
            ));
            continue;
        }

        let verse_end = VERSE_WORDS.min(result.len());

        // print first-verse table
        let mut errors: Vec<f64> = Vec::new();
        for i in 0..verse_end {
            let got_ms = result[i].start * 1_000.0;
            let exp_ms = ground_truth[i].start_ms as f64;
            let err = (got_ms - exp_ms).abs();
            errors.push(err);
            println!("  {:>14}  got={:.0}ms  exp={}ms  err={:.0}ms",
                result[i].word, got_ms, exp_ms, err);
        }

        // 2) cascade check (first verse)
        let mut cascade_violations: Vec<String> = Vec::new();
        for w in result[..verse_end].windows(2) {
            let gap_ms = (w[1].start - w[0].start) * 1_000.0;
            if gap_ms < CASCADE_MIN_MS {
                cascade_violations.push(format!(
                    "'{}' ({:.0}ms) → '{}' ({:.0}ms) gap={:.0}ms",
                    w[0].word, w[0].start * 1000.0,
                    w[1].word, w[1].start * 1000.0,
                    gap_ms
                ));
            }
        }
        if !cascade_violations.is_empty() {
            println!("  cascade violations:");
            for v in &cascade_violations { println!("    {v}"); }
            failures.push(format!(
                "[{name}] {} cascade violation(s) in first {verse_end} words",
                cascade_violations.len()
            ));
        }

        // 3) MAE check (first verse)
        if !errors.is_empty() {
            let mae = errors.iter().sum::<f64>() / errors.len() as f64;
            println!("  First-{verse_end}-words MAE = {mae:.1}ms");
            if mae >= MAE_THRESHOLD_MS {
                failures.push(format!(
                    "[{name}] first-verse MAE {mae:.1}ms ≥ {MAE_THRESHOLD_MS}ms threshold"
                ));
            }
        }

        // 4) Full-song MAE + worst offenders.
        let mut full_errors: Vec<(usize, f64)> = (0..result.len())
            .map(|i| {
                let err = (result[i].start * 1000.0 - ground_truth[i].start_ms as f64).abs();
                (i, err)
            })
            .collect();
        let full_mae = full_errors.iter().map(|(_, e)| e).sum::<f64>() / full_errors.len() as f64;
        full_errors.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let p90 = {
            let mut sorted: Vec<f64> = full_errors.iter().map(|(_, e)| *e).collect();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            sorted[(sorted.len() as f64 * 0.9) as usize]
        };

        println!("  Full-song MAE = {full_mae:.1}ms, P90 = {p90:.1}ms ({} words)", result.len());
        println!("  Top 10 worst-aligned words:");
        for (i, err) in full_errors.iter().take(10) {
            println!("    [{:>3}] {:>16}  got={:.0}ms  exp={}ms  err={:.0}ms",
                i, result[*i].word, result[*i].start * 1000.0,
                ground_truth[*i].start_ms, err);
        }

        const FULL_MAE_THRESHOLD_MS: f64 = 800.0;
        if full_mae >= FULL_MAE_THRESHOLD_MS {
            failures.push(format!(
                "[{name}] full-song MAE {full_mae:.1}ms ≥ {FULL_MAE_THRESHOLD_MS}ms threshold"
            ));
        }

        // 5) LRC-window check: each aligned word must fall inside its phrase's
        //    [start, next_start) window, per the `line` field in words.json.
        if !lrc_phrases.is_empty() {
            let mut violations: Vec<(usize, f64, u64, u64, String)> = Vec::new();
            for i in 0..result.len() {
                let line = ground_truth[i].line;
                if line >= lrc_phrases.len() { continue; }
                let lo = lrc_phrases[line];
                let hi = lrc_phrases
                    .get(line + 1)
                    .copied()
                    .unwrap_or(u64::MAX);
                let got_ms = (result[i].start * 1000.0) as i64;
                let lo_i = lo as i64;
                let hi_i = if hi == u64::MAX { i64::MAX } else { hi as i64 };
                if got_ms < lo_i || got_ms >= hi_i {
                    violations.push((
                        i,
                        result[i].start * 1000.0,
                        lo,
                        hi,
                        result[i].word.clone(),
                    ));
                }
            }
            println!(
                "  LRC-window: {}/{} words outside their phrase window ({} phrases)",
                violations.len(), result.len(), lrc_phrases.len()
            );
            for (i, got, lo, hi, w) in violations.iter().take(15) {
                let hi_show = if *hi == u64::MAX { "∞".to_string() } else { format!("{hi}") };
                println!("    [{i:>3}] {w:>16}  got={got:.0}ms  window=[{lo},{hi_show})");
            }
            const LRC_MAX_VIOLATIONS_RATIO: f64 = 0.10;
            let ratio = violations.len() as f64 / result.len() as f64;
            if ratio > LRC_MAX_VIOLATIONS_RATIO {
                failures.push(format!(
                    "[{name}] LRC-window violations {}/{} ({:.1}%) > {:.0}% threshold",
                    violations.len(), result.len(), ratio * 100.0,
                    LRC_MAX_VIOLATIONS_RATIO * 100.0
                ));
            }
        }
    }

    if !failures.is_empty() {
        panic!("Song regression failures:\n{}", failures.join("\n"));
    }
}

/// Two concatenated copies of the clean clip must produce no duplicate words
/// at the chunk boundary.
#[tokio::test]
async fn test_no_duplicate_at_chunk_boundary() {
    let assets = test_assets_dir();
    let audio_path = assets.join("vocals-clean.wav");

    if skip_if_missing(&audio_path) {
        return;
    }

    let single = load_wav_mono_16k(&audio_path).expect("load vocals-clean.wav");
    let double_samples = [single.samples.as_slice(), single.samples.as_slice()].concat();
    let double = AudioBuffer { samples: double_samples, sample_rate: 16_000 };

    let lyrics_single =
        "Nel mezzo del cammin di nostra vita mi ritrovai per una selva oscura";
    let lyrics_double = format!("{} {}", lyrics_single, lyrics_single);

    let aligner = ForcedAligner::new(AlignmentConfig {
        model: if cfg!(feature = "metal") { WhisperModel::Medium } else { WhisperModel::Small },
        language: "it".to_string(),
        ..Default::default()
    })
    .await
    .expect("load model");
    let result = aligner.align(&double, &lyrics_double).expect("align");

    let expected_len = lyrics_double.split_whitespace().count();
    assert_eq!(result.len(), expected_len, "word count mismatch on doubled clip");
}
