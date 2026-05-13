use ndarray::ArrayView2;

use crate::AlignError;

/// Per-token frame span (`start_frame`, `end_frame`) inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenSpan {
    pub start: usize,
    pub end: usize,
}

/// CTC forced alignment via Viterbi over an extended target sequence with
/// blanks inserted between every emitted token (and at the boundaries).
///
/// `log_probs` is `[T, V]` log-softmax output. `targets` is the non-blank token
/// id sequence (e.g. characters + word delimiters). `blank_id` is conventionally
/// the wav2vec2 `<pad>` id.
///
/// Returns one [`TokenSpan`] per entry in `targets`. Frames are inclusive and
/// guaranteed monotonically non-decreasing.
///
/// References: Graves 2006, torchaudio `forced_align`.
pub fn forced_align(
    log_probs: ArrayView2<f32>,
    targets: &[u32],
    blank_id: u32,
) -> Result<Vec<TokenSpan>, AlignError> {
    let t = log_probs.shape()[0];
    let v = log_probs.shape()[1];
    let n = targets.len();

    if n == 0 {
        return Ok(Vec::new());
    }
    if t == 0 {
        return Err(AlignError::Ctc("zero time frames in log_probs".into()));
    }
    if v == 0 {
        return Err(AlignError::Ctc("zero vocab dim in log_probs".into()));
    }

    if n > t {
        return Err(AlignError::Ctc(format!(
            "target sequence too long ({} tokens, {} frames)",
            n, t
        )));
    }

    // Extended targets: blank between every entry plus at both ends.
    // ext[0] = blank, ext[1] = targets[0], ext[2] = blank, ext[3] = targets[1], ...
    // Length 2N+1.
    let m = 2 * n + 1;
    let ext: Vec<u32> = (0..m)
        .map(|i| if i % 2 == 0 { blank_id } else { targets[i / 2] })
        .collect();

    let neg_inf = f32::NEG_INFINITY;
    let mut alpha = vec![neg_inf; t * m];
    let mut back = vec![0u8; t * m]; // 0=stay, 1=prev, 2=skip
    let idx = |row: usize, col: usize| row * m + col;

    // Initialise t=0: can only start at ext[0]=blank or ext[1]=targets[0].
    alpha[idx(0, 0)] = log_probs[[0, blank_id as usize]];
    if m >= 2 {
        alpha[idx(0, 1)] = log_probs[[0, ext[1] as usize]];
    }

    for time in 1..t {
        // Constrain the valid state band so we cannot land short of the target.
        // At time `time` we must reach a state s with `s >= m - 2*(t-1-time) - 1`
        // and `s <= 2*time + 1` (typical CTC constraints).
        let lo = m.saturating_sub(2 * (t - time));
        let hi = (2 * time + 2).min(m);

        for s in lo..hi {
            let emit = log_probs[[time, ext[s] as usize]];
            // Three predecessors:
            //  - stay (alpha[t-1][s])
            //  - prev (alpha[t-1][s-1])
            //  - skip (alpha[t-1][s-2]) if ext[s] != blank and ext[s] != ext[s-2]
            let stay = alpha[idx(time - 1, s)];
            let prev = if s >= 1 { alpha[idx(time - 1, s - 1)] } else { neg_inf };
            let skip = if s >= 2 && ext[s] != blank_id && ext[s] != ext[s - 2] {
                alpha[idx(time - 1, s - 2)]
            } else {
                neg_inf
            };

            let (best, choice) = max3(stay, prev, skip);
            if best == neg_inf {
                continue;
            }
            alpha[idx(time, s)] = best + emit;
            back[idx(time, s)] = choice;
        }
    }

    // Pick best terminal state — must be one of the last two (final blank or final token).
    let final_a = alpha[idx(t - 1, m - 1)];
    let final_b = if m >= 2 { alpha[idx(t - 1, m - 2)] } else { neg_inf };
    let mut s = if final_a >= final_b { m - 1 } else { m - 2 };
    if alpha[idx(t - 1, s)] == neg_inf {
        return Err(AlignError::Ctc(
            "forced alignment failed: no valid terminal path".into(),
        ));
    }

    // Backtrack — record extended-state per frame.
    let mut state_per_frame = vec![0usize; t];
    state_per_frame[t - 1] = s;
    for time in (1..t).rev() {
        let choice = back[idx(time, s)];
        s = match choice {
            0 => s,
            1 => s.saturating_sub(1),
            2 => s.saturating_sub(2),
            _ => s,
        };
        state_per_frame[time - 1] = s;
    }

    // Convert per-frame extended-state into per-target-token spans.
    let mut spans: Vec<Option<TokenSpan>> = vec![None; n];
    for (time, &state) in state_per_frame.iter().enumerate() {
        if state % 2 == 1 {
            let token_idx = state / 2;
            match &mut spans[token_idx] {
                None => spans[token_idx] = Some(TokenSpan { start: time, end: time }),
                Some(span) => span.end = time,
            }
        }
    }

    // Any target that never appeared in the path gets a degenerate span.
    let mut out = Vec::with_capacity(n);
    let mut last_end = 0usize;
    for s in spans {
        let span = s.unwrap_or(TokenSpan { start: last_end, end: last_end });
        last_end = span.end;
        out.push(span);
    }

    Ok(out)
}

fn max3(a: f32, b: f32, c: f32) -> (f32, u8) {
    if a >= b && a >= c {
        (a, 0)
    } else if b >= c {
        (b, 1)
    } else {
        (c, 2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::Array2;

    fn log_one_hot(target: u32, vocab: usize, sharpness: f32) -> Vec<f32> {
        // Build log-softmax-ish row: target gets near-0, rest get -sharpness.
        (0..vocab)
            .map(|i| if i as u32 == target { 0.0 } else { -sharpness })
            .collect()
    }

    #[test]
    fn aligns_three_tokens_in_five_frames() {
        // Vocab: blank=0, a=1, b=2, c=3. Targets: a b c.
        let vocab = 4;
        let blank = 0u32;
        let frames: Vec<u32> = vec![1, 0, 2, 0, 3];
        let mut data = Vec::with_capacity(frames.len() * vocab);
        for f in &frames {
            data.extend(log_one_hot(*f, vocab, 5.0));
        }
        let log_probs = Array2::from_shape_vec((frames.len(), vocab), data).unwrap();

        let spans = forced_align(log_probs.view(), &[1, 2, 3], blank).unwrap();
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0], TokenSpan { start: 0, end: 0 });
        assert_eq!(spans[1], TokenSpan { start: 2, end: 2 });
        assert_eq!(spans[2], TokenSpan { start: 4, end: 4 });
    }

    #[test]
    fn aligns_with_repeated_tokens() {
        // Targets: a a (needs blank between them).
        let vocab = 3;
        let blank = 0u32;
        let frames: Vec<u32> = vec![1, 1, 0, 1, 1];
        let mut data = Vec::with_capacity(frames.len() * vocab);
        for f in &frames {
            data.extend(log_one_hot(*f, vocab, 5.0));
        }
        let log_probs = Array2::from_shape_vec((frames.len(), vocab), data).unwrap();

        let spans = forced_align(log_probs.view(), &[1, 1], blank).unwrap();
        assert_eq!(spans.len(), 2);
        assert!(spans[0].end < spans[1].start);
    }

    #[test]
    fn rejects_target_longer_than_frames() {
        let log_probs = Array2::<f32>::zeros((2, 3));
        let err = forced_align(log_probs.view(), &[1, 2, 1], 0).unwrap_err();
        match err {
            AlignError::Ctc(_) => {}
            other => panic!("expected Ctc error, got {:?}", other),
        }
    }
}
