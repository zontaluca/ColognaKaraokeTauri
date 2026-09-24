//! Engine-independent word timing: turns the timed words produced by an
//! aligner (wav2vec2 CTC) or a recognizer (Parakeet) into words.json entries
//! attributed to lyric lines, and generates an LRC when the lyrics had none.
//!
//! Everything here is pure (no model, no I/O) so it can be unit-tested.

use serde_json::{json, Value};

/// A lyric line to be timed. `ts_ms` is the LRC line timestamp, `None` for
/// plain (unsynced) lyrics.
#[derive(Debug, Clone, PartialEq)]
pub struct LyricLine {
    pub text: String,
    pub ts_ms: Option<u64>,
}

/// Lyrics prepared for alignment.
#[derive(Debug, Clone)]
pub struct LyricsInput {
    pub lines: Vec<LyricLine>,
    /// True when every line carries an LRC timestamp.
    pub synced: bool,
}

impl LyricsInput {
    /// All line texts joined with spaces (the text given to the aligner).
    pub fn joined_text(&self) -> String {
        self.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join(" ")
    }
}

/// Parse fetched lyrics. Synced LRC keeps its line timestamps; plain lyrics
/// become one line per non-empty text line, dropping section markers such as
/// "[Chorus]". Returns `None` when there is nothing to align.
pub fn parse_lyrics(text: &str) -> Option<LyricsInput> {
    let synced = crate::lyrics::parse_lrc(text);
    if !synced.is_empty() {
        let lines = synced
            .into_iter()
            .map(|l| LyricLine { text: l.text, ts_ms: Some(l.ts_ms) })
            .collect();
        return Some(LyricsInput { lines, synced: true });
    }
    let lines: Vec<LyricLine> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !is_section_marker(l))
        .map(|l| LyricLine { text: l.to_string(), ts_ms: None })
        .collect();
    if lines.is_empty() {
        None
    } else {
        Some(LyricsInput { lines, synced: false })
    }
}

fn is_section_marker(line: &str) -> bool {
    line.starts_with('[') && line.ends_with(']')
}

/// A word with song-relative times in seconds, from any engine.
#[derive(Debug, Clone, PartialEq)]
pub struct TimedWord {
    pub word: String,
    pub start: f64,
    pub end: f64,
    /// Engine confidence in [0, 1] when available (CTC mean emission prob).
    pub confidence: Option<f32>,
}

/// One words.json entry.
#[derive(Debug, Clone, PartialEq)]
pub struct WordEntry {
    pub word: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub line: Option<usize>,
    pub score: Option<f32>,
    /// Timing interpolated from neighbouring words rather than measured.
    pub estimated: bool,
}

impl WordEntry {
    pub fn to_json(&self) -> Value {
        let mut obj = serde_json::Map::new();
        obj.insert("word".into(), Value::String(self.word.clone()));
        obj.insert("start_ms".into(), json!(self.start_ms));
        obj.insert("end_ms".into(), json!(self.end_ms));
        if let Some(l) = self.line {
            obj.insert("line".into(), json!(l));
        }
        if let Some(s) = self.score {
            obj.insert("score".into(), json!((s * 1000.0).round() / 1000.0));
        }
        if self.estimated {
            obj.insert("estimated".into(), json!(true));
        }
        Value::Object(obj)
    }
}

/// Split a flat, in-order word stream into per-line groups of `counts[i]` words.
pub fn split_by_counts<T>(words: Vec<T>, counts: &[usize]) -> Vec<Vec<T>> {
    let mut it = words.into_iter();
    counts.iter().map(|&n| it.by_ref().take(n).collect()).collect()
}

/// Largest spread (median absolute deviation) of per-line offsets for which
/// the LRC timing is considered consistent with the audio.
const MAX_OFFSET_SPREAD_MS: i64 = 600;
const MIN_OFFSET_SAMPLES: usize = 3;

/// Median offset (ms, audio minus LRC) between each line's first aligned word
/// and its LRC timestamp. `None` when there are too few lines or the offsets
/// disagree too much for the LRC timing to be trusted.
pub fn estimate_lrc_offset_ms(lines: &[LyricLine], per_line: &[Vec<TimedWord>]) -> Option<i64> {
    let mut diffs: Vec<i64> = lines
        .iter()
        .zip(per_line)
        .filter_map(|(line, words)| {
            let ts = line.ts_ms? as i64;
            let first = words.first()?;
            Some((first.start * 1000.0).round() as i64 - ts)
        })
        .collect();
    if diffs.len() < MIN_OFFSET_SAMPLES {
        return None;
    }
    let offset = median_i64(&mut diffs);
    let mut deviations: Vec<i64> = diffs.iter().map(|d| (d - offset).abs()).collect();
    let spread = median_i64(&mut deviations);
    (spread <= MAX_OFFSET_SPREAD_MS).then_some(offset)
}

fn median_i64(values: &mut [i64]) -> i64 {
    values.sort_unstable();
    values[values.len() / 2]
}

/// Margin added around each line's LRC span when re-aligning it on its own.
const WINDOW_MARGIN_SEC: f64 = 1.0;
/// Span given to the last line (no following timestamp to bound it).
const LAST_LINE_SPAN_SEC: f64 = 12.0;

/// Per-line alignment windows in song seconds: from the line's (offset
/// corrected) timestamp to the next line's, widened by a margin and clipped
/// to `[0, end_sec]`. `None` for lines without a timestamp.
pub fn line_windows(lines: &[LyricLine], offset_ms: i64, end_sec: f64) -> Vec<Option<(f64, f64)>> {
    let at = |ts: u64| (ts as i64 + offset_ms) as f64 / 1000.0;
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let start = at(line.ts_ms?);
            let next = lines.get(i + 1).and_then(|l| l.ts_ms).map(at);
            let end = next.unwrap_or(start + LAST_LINE_SPAN_SEC);
            let w0 = (start - WINDOW_MARGIN_SEC).max(0.0);
            let w1 = (end + WINDOW_MARGIN_SEC).min(end_sec);
            (w1 > w0).then_some((w0, w1))
        })
        .collect()
}

/// Mean confidence of a group of words (0 when unknown).
pub fn mean_confidence(words: &[TimedWord]) -> f32 {
    let vals: Vec<f32> = words.iter().filter_map(|w| w.confidence).collect();
    if vals.is_empty() {
        0.0
    } else {
        vals.iter().sum::<f32>() / vals.len() as f32
    }
}

fn clean_token(raw: &str) -> String {
    raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '-')
        .to_string()
}

/// Build words.json entries from per-line timed words.
///
/// - Word ends never cross the next line's (offset-corrected) timestamp, so a
///   single misaligned word can't pull the next line's highlight forward.
/// - Runs of low-confidence words between two confident words of the same
///   line are re-timed evenly (by length) inside that gap.
/// - Starts are made globally monotonic.
pub fn build_entries(
    lines: &[LyricLine],
    per_line: Vec<Vec<TimedWord>>,
    offset_ms: Option<i64>,
) -> Vec<WordEntry> {
    let low_threshold = low_confidence_threshold(&per_line);
    let offset = offset_ms.unwrap_or(0);
    let mut out: Vec<WordEntry> = Vec::new();

    for (line_idx, words) in per_line.into_iter().enumerate() {
        let line_end_ms = lines
            .get(line_idx + 1)
            .and_then(|l| l.ts_ms)
            .map(|ts| (ts as i64 + offset).max(0) as u64)
            .unwrap_or(u64::MAX);
        let mut line_entries: Vec<WordEntry> = Vec::with_capacity(words.len());
        for w in words {
            let clean = clean_token(&w.word);
            if clean.is_empty() {
                continue;
            }
            let start_ms = ((w.start * 1000.0).max(0.0)).round() as u64;
            let mut end_ms = ((w.end * 1000.0).max(0.0)).round() as u64;
            if end_ms <= start_ms {
                end_ms = start_ms + 50;
            }
            // Clamp into LRC neighbour window so a single misaligned word can't
            // pull the next line's highlight forward.
            let start_ms = start_ms.min(line_end_ms.saturating_sub(50));
            let end_ms = end_ms.min(line_end_ms);
            line_entries.push(WordEntry {
                word: clean,
                start_ms,
                end_ms,
                line: Some(line_idx),
                score: w.confidence,
                estimated: false,
            });
        }
        if let Some(threshold) = low_threshold {
            retime_low_confidence_runs(&mut line_entries, threshold);
        }
        out.extend(line_entries);
    }

    // Global monotonicity guard.
    let mut last = 0_u64;
    for w in out.iter_mut() {
        if w.start_ms < last {
            w.start_ms = last;
        }
        if w.end_ms <= w.start_ms {
            w.end_ms = w.start_ms + 50;
        }
        last = w.start_ms;
    }
    out
}

/// A word counts as unreliable below a quarter of the song's median confidence
/// (relative, so heavily processed vocals with uniformly low scores are left
/// alone), with an absolute floor.
fn low_confidence_threshold(per_line: &[Vec<TimedWord>]) -> Option<f32> {
    let mut all: Vec<f32> = per_line.iter().flatten().filter_map(|w| w.confidence).collect();
    if all.len() < 8 {
        return None;
    }
    all.sort_by(|a, b| a.total_cmp(b));
    let median = all[all.len() / 2];
    Some((median * 0.25).max(0.02))
}

/// Minimum time given to each re-timed word; tighter gaps are left as aligned.
const MIN_RETIMED_WORD_MS: u64 = 60;

fn retime_low_confidence_runs(entries: &mut [WordEntry], threshold: f32) {
    let is_low = |e: &WordEntry| e.score.map_or(false, |s| s < threshold);
    let mut i = 0;
    while i < entries.len() {
        if !is_low(&entries[i]) {
            i += 1;
            continue;
        }
        let run_start = i;
        while i < entries.len() && is_low(&entries[i]) {
            i += 1;
        }
        let run_end = i; // exclusive
        // Needs a confident anchor on both sides within the line.
        if run_start == 0 || run_end >= entries.len() {
            continue;
        }
        let gap_start = entries[run_start - 1].end_ms;
        let gap_end = entries[run_end].start_ms;
        let run_len = (run_end - run_start) as u64;
        if gap_end <= gap_start || gap_end - gap_start < MIN_RETIMED_WORD_MS * run_len {
            continue;
        }
        let weights: Vec<u64> = entries[run_start..run_end]
            .iter()
            .map(|e| e.word.chars().count().max(1) as u64)
            .collect();
        let total: u64 = weights.iter().sum();
        let span = gap_end - gap_start;
        let mut cursor = gap_start;
        for (k, e) in entries[run_start..run_end].iter_mut().enumerate() {
            let len = span * weights[k] / total;
            e.start_ms = cursor;
            e.end_ms = if k + 1 == weights.len() { gap_end } else { cursor + len };
            e.estimated = true;
            cursor = e.end_ms;
        }
    }
}

/// Build a synced LRC from the timed words of plain lyrics: each line starts at
/// its first word. Lines that got no word inherit the previous line's time so
/// the line order (and thus words.json `line` indices) is preserved.
pub fn generate_lrc(lines: &[LyricLine], entries: &[WordEntry]) -> String {
    let mut first_start: Vec<Option<u64>> = vec![None; lines.len()];
    for e in entries {
        if let Some(l) = e.line {
            if let Some(slot) = first_start.get_mut(l) {
                if slot.map_or(true, |s| e.start_ms < s) {
                    *slot = Some(e.start_ms);
                }
            }
        }
    }
    let mut out = String::new();
    let mut last = 0_u64;
    for (line, start) in lines.iter().zip(first_start) {
        let ts = start.unwrap_or(last).max(last);
        last = ts;
        out.push_str(&format_lrc_timestamp(ts));
        out.push_str(&line.text);
        out.push('\n');
    }
    out
}

/// Same as [`generate_lrc`] for words already stored in words.json.
pub fn generate_lrc_from_json(lines: &[LyricLine], words: &Value) -> Option<String> {
    let arr = words.as_array()?;
    let entries: Vec<WordEntry> = arr
        .iter()
        .filter_map(|w| {
            Some(WordEntry {
                word: w.get("word")?.as_str()?.to_string(),
                start_ms: w.get("start_ms")?.as_u64()?,
                end_ms: w.get("end_ms").and_then(Value::as_u64).unwrap_or(0),
                line: w.get("line").and_then(Value::as_u64).map(|l| l as usize),
                score: None,
                estimated: false,
            })
        })
        .collect();
    (!entries.is_empty()).then(|| generate_lrc(lines, &entries))
}

/// `[mm:ss.xx]` (centiseconds), the format parsed by both backend and frontend.
pub fn format_lrc_timestamp(ms: u64) -> String {
    let centis = (ms + 5) / 10;
    format!("[{:02}:{:02}.{:02}]", centis / 6000, (centis / 100) % 60, centis % 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tw(word: &str, start: f64, end: f64, conf: f32) -> TimedWord {
        TimedWord { word: word.into(), start, end, confidence: Some(conf) }
    }

    fn line(text: &str, ts: Option<u64>) -> LyricLine {
        LyricLine { text: text.into(), ts_ms: ts }
    }

    #[test]
    fn parses_synced_and_plain_lyrics() {
        let synced = parse_lyrics("[00:01.00] uno due\n[00:03.00] tre").unwrap();
        assert!(synced.synced);
        assert_eq!(synced.lines[1], line("tre", Some(3000)));

        let plain = parse_lyrics("[Chorus]\nuno due\n\n  tre  \n").unwrap();
        assert!(!plain.synced);
        assert_eq!(plain.lines, vec![line("uno due", None), line("tre", None)]);

        assert!(parse_lyrics("  \n[Intro]\n").is_none());
    }

    #[test]
    fn splits_stream_by_line_counts() {
        let groups = split_by_counts(vec![1, 2, 3, 4, 5], &[2, 0, 3]);
        assert_eq!(groups, vec![vec![1, 2], vec![], vec![3, 4, 5]]);
    }

    #[test]
    fn estimates_consistent_offset_and_rejects_noise() {
        let lines = vec![line("a", Some(1000)), line("b", Some(5000)), line("c", Some(9000)), line("d", Some(13000))];
        let late = |extra: f64| vec![
            vec![tw("a", 2.5, 3.0, 0.5)],
            vec![tw("b", 6.5 + extra, 7.0 + extra, 0.5)],
            vec![tw("c", 10.4, 11.0, 0.5)],
            vec![tw("d", 14.6, 15.0, 0.5)],
        ];
        assert_eq!(estimate_lrc_offset_ms(&lines, &late(0.0)), Some(1500));
        let noisy = vec![
            vec![tw("a", 2.0, 3.0, 0.5)],
            vec![tw("b", 12.0, 13.0, 0.5)],
            vec![tw("c", 3.0, 4.0, 0.5)],
            vec![tw("d", 30.0, 31.0, 0.5)],
        ];
        assert_eq!(estimate_lrc_offset_ms(&lines, &noisy), None);
    }

    #[test]
    fn windows_follow_shifted_timestamps() {
        let lines = vec![line("a", Some(1000)), line("b", Some(5000))];
        let w = line_windows(&lines, 500, 100.0);
        assert_eq!(w[0], Some((0.5, 6.5)));
        assert_eq!(w[1], Some((4.5, 18.5)));
    }

    #[test]
    fn clamps_to_offset_corrected_next_line() {
        let lines = vec![line("a b", Some(1000)), line("c", Some(3000))];
        // LRC is 2 s early: the raw next-line clamp (3 s) would squash "b".
        let per_line = vec![vec![tw("a", 3.0, 3.5, 0.8), tw("b", 4.0, 4.6, 0.8)], vec![tw("c", 5.0, 5.5, 0.8)]];
        let entries = build_entries(&lines, per_line, Some(2000));
        assert_eq!((entries[1].start_ms, entries[1].end_ms), (4000, 4600));
    }

    #[test]
    fn retimes_low_confidence_run_between_anchors() {
        let lines = vec![line("uno due tre quattro", None), line("x x x x x x", None)];
        let per_line = vec![
            vec![tw("uno", 1.0, 1.4, 0.8), tw("due", 1.4, 1.45, 0.01), tw("tre", 1.45, 1.5, 0.01), tw("quattro", 3.0, 3.5, 0.8)],
            (0..6).map(|i| tw("x", 4.0 + i as f64, 4.5 + i as f64, 0.7)).collect(),
        ];
        let e = build_entries(&lines, per_line, None);
        assert!(e[1].estimated && e[2].estimated && !e[0].estimated);
        assert_eq!(e[1].start_ms, 1400);
        assert_eq!(e[2].end_ms, 3000);
        assert_eq!(e[1].end_ms, e[2].start_ms);
    }

    #[test]
    fn generates_lrc_keeping_every_line() {
        let lines = vec![line("uno", None), line("...", None), line("due", None)];
        let entries = vec![
            WordEntry { word: "uno".into(), start_ms: 1234, end_ms: 1500, line: Some(0), score: None, estimated: false },
            WordEntry { word: "due".into(), start_ms: 65_000, end_ms: 65_500, line: Some(2), score: None, estimated: false },
        ];
        let lrc = generate_lrc(&lines, &entries);
        assert_eq!(lrc, "[00:01.23]uno\n[00:01.23]...\n[01:05.00]due\n");
        // Round-trips through the LRC parser with the same line order.
        let parsed = crate::lyrics::parse_lrc(&lrc);
        assert_eq!(parsed.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), vec!["uno", "...", "due"]);
    }
}
