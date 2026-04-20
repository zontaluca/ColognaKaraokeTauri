//! Dynamic Time Warping for aligning token sequences to audio frames.
//!
//! Input:  cost matrix `C[i][j]` = dissimilarity between text token `i` and
//!         audio frame `j`. Typically `1 - attention_weight[i][j]`.
//! Output: optimal monotone path from (0,0) to (n-1, m-1).

/// Run DTW and return the optimal path as (token_idx, frame_idx) pairs.
///
/// `cost` must be non-empty and rectangular. Allowed steps: diagonal, down,
/// right (standard DTW).
pub fn dtw(cost: &[Vec<f32>]) -> Vec<(usize, usize)> {
    let n = cost.len();
    let m = cost[0].len();

    let mut dp = vec![vec![f32::INFINITY; m]; n];
    dp[0][0] = cost[0][0];
    for j in 1..m {
        dp[0][j] = dp[0][j - 1] + cost[0][j];
    }
    for i in 1..n {
        dp[i][0] = dp[i - 1][0] + cost[i][0];
    }
    for i in 1..n {
        for j in 1..m {
            let prev = dp[i - 1][j - 1].min(dp[i - 1][j]).min(dp[i][j - 1]);
            dp[i][j] = cost[i][j] + prev;
        }
    }

    // Traceback from (n-1, m-1) to (0, 0).
    let mut path = Vec::with_capacity(n + m);
    let mut i = n - 1;
    let mut j = m - 1;
    path.push((i, j));
    while i > 0 || j > 0 {
        if i == 0 {
            j -= 1;
        } else if j == 0 {
            i -= 1;
        } else {
            let diag = dp[i - 1][j - 1];
            let up = dp[i - 1][j];
            let left = dp[i][j - 1];
            let min = diag.min(up).min(left);
            if (diag - min).abs() < f32::EPSILON {
                i -= 1;
                j -= 1;
            } else if (up - min).abs() < f32::EPSILON {
                i -= 1;
            } else {
                j -= 1;
            }
        }
        path.push((i, j));
    }
    path.reverse();
    path
}

/// Given a DTW path, return for each token index the start and end frame index.
///
/// `start[i]` = first frame assigned to token i
/// `end[i]`   = last frame assigned to token i (inclusive)
pub fn path_to_token_spans(path: &[(usize, usize)], n_tokens: usize) -> Vec<(usize, usize)> {
    let mut spans = vec![(usize::MAX, 0usize); n_tokens];
    for &(tok, frame) in path {
        let (s, e) = &mut spans[tok];
        if frame < *s {
            *s = frame;
        }
        if frame > *e {
            *e = frame;
        }
    }
    // Fix any tokens that never appeared (shouldn't happen with a valid path).
    for span in &mut spans {
        if span.0 == usize::MAX {
            span.0 = 0;
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_sequence() {
        // Perfect diagonal cost matrix → path should be diagonal.
        let n = 4;
        let mut cost = vec![vec![1.0f32; n]; n];
        for i in 0..n {
            cost[i][i] = 0.0;
        }
        let path = dtw(&cost);
        // Must start at (0,0) and end at (n-1, n-1).
        assert_eq!(path.first(), Some(&(0, 0)));
        assert_eq!(path.last(), Some(&(n - 1, n - 1)));
    }

    #[test]
    fn monotone() {
        let cost: Vec<Vec<f32>> = (0..5)
            .map(|i| (0..10).map(|j| if j == i * 2 { 0.0 } else { 1.0 }).collect())
            .collect();
        let path = dtw(&cost);
        // Token indices must be non-decreasing.
        for w in path.windows(2) {
            assert!(w[1].0 >= w[0].0);
        }
    }

    #[test]
    fn spans_cover_all_tokens() {
        let cost = vec![vec![0.5f32; 20]; 5];
        let path = dtw(&cost);
        let spans = path_to_token_spans(&path, 5);
        assert_eq!(spans.len(), 5);
        for (s, e) in &spans {
            assert!(s <= e);
        }
    }
}
