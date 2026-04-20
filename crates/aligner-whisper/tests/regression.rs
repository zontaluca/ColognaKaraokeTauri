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
use aligner_whisper::{AlignError, AlignmentConfig, ForcedAligner, WhisperModel};
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
