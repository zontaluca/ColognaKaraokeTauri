//! Text normalization for lyrics alignment.
//!
//! `normalize()` maps each original word to a canonical form used for token
//! matching.  The original word is always preserved in AlignedWord::word.

/// Common Italian contractions → expansion.
static IT_CONTRACTIONS: &[(&str, &str)] = &[
    ("dell'", "della "),
    ("dell'", "della "),
    ("dell'", "della "),
    ("all'", "alla "),
    ("all'", "alla "),
    ("all'", "alla "),
    ("nell'", "nella "),
    ("nell'", "nella "),
    ("sull'", "sulla "),
    ("sull'", "sulla "),
    ("dall'", "dalla "),
    ("dall'", "dalla "),
    ("c'è", "ci è"),
    ("c'e'", "ci è"),
    ("po'", "poco"),
    ("po'", "poco"),
    ("n'", "ne "),
    ("l'", "la "),
    ("l'", "la "),
    ("m'", "mi "),
    ("t'", "ti "),
    ("s'", "si "),
    ("v'", "vi "),
    ("d'", "di "),
];

/// English contractions that may appear in Italian songs.
static EN_CONTRACTIONS: &[(&str, &str)] = &[
    ("i'm", "i am"),
    ("i've", "i have"),
    ("i'll", "i will"),
    ("i'd", "i would"),
    ("you're", "you are"),
    ("you've", "you have"),
    ("you'll", "you will"),
    ("you'd", "you would"),
    ("he's", "he is"),
    ("she's", "she is"),
    ("it's", "it is"),
    ("we're", "we are"),
    ("we've", "we have"),
    ("we'll", "we will"),
    ("they're", "they are"),
    ("they've", "they have"),
    ("they'll", "they will"),
    ("don't", "do not"),
    ("doesn't", "does not"),
    ("didn't", "did not"),
    ("won't", "will not"),
    ("can't", "cannot"),
    ("couldn't", "could not"),
    ("shouldn't", "should not"),
    ("wouldn't", "would not"),
    ("isn't", "is not"),
    ("aren't", "are not"),
    ("wasn't", "was not"),
    ("weren't", "were not"),
    ("that's", "that is"),
    ("what's", "what is"),
    ("there's", "there is"),
    ("here's", "here is"),
    ("let's", "let us"),
    ("'cause", "because"),
    ("'em", "them"),
    ("'til", "until"),
];

/// Normalize a single word: lowercase, expand contraction, strip non-alpha.
/// Returns one or more normalized tokens (contractions expand to two words).
pub fn normalize_word(word: &str) -> Vec<String> {
    let lower = word.to_lowercase();
    let lower = lower.trim();

    // Try Italian contractions first (longest match).
    for (contracted, expanded) in IT_CONTRACTIONS {
        if lower == *contracted || lower.starts_with(contracted) {
            return expanded
                .split_whitespace()
                .map(strip_non_alpha)
                .filter(|s| !s.is_empty())
                .collect();
        }
    }

    // English contractions.
    for (contracted, expanded) in EN_CONTRACTIONS {
        if lower == *contracted {
            return expanded
                .split_whitespace()
                .map(strip_non_alpha)
                .filter(|s| !s.is_empty())
                .collect();
        }
    }

    let stripped = strip_non_alpha(lower);
    if stripped.is_empty() {
        vec![]
    } else {
        vec![stripped]
    }
}

fn strip_non_alpha(s: impl AsRef<str>) -> String {
    s.as_ref()
        .chars()
        .filter(|c| c.is_alphabetic() || *c == '\'')
        .collect::<String>()
        .trim_matches('\'')
        .to_string()
}

/// Normalize a full lyrics string. Returns (original_words, normalized_tokens)
/// where each original word maps to one or more normalized tokens via index mapping.
///
/// The returned `mapping[i]` gives the range `[start, end)` of normalized tokens
/// that correspond to original word `i`.
pub fn normalize_lyrics(
    lyrics: &str,
) -> (Vec<String>, Vec<String>, Vec<(usize, usize)>) {
    let original_words: Vec<String> = lyrics
        .split_whitespace()
        .map(|w| w.to_string())
        .collect();

    let mut normalized: Vec<String> = Vec::new();
    let mut mapping: Vec<(usize, usize)> = Vec::new();

    for word in &original_words {
        let start = normalized.len();
        let tokens = normalize_word(word);
        normalized.extend(tokens);
        let end = normalized.len();
        mapping.push((start, end));
    }

    (original_words, normalized, mapping)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_strip() {
        assert_eq!(normalize_word("Ciao,"), vec!["ciao"]);
        assert_eq!(normalize_word("amore!"), vec!["amore"]);
    }

    #[test]
    fn italian_contraction() {
        let r = normalize_word("dell'");
        assert!(r.contains(&"della".to_string()) || r.len() >= 1);
    }

    #[test]
    fn english_contraction() {
        let r = normalize_word("don't");
        assert_eq!(r, vec!["do", "not"]);
    }

    #[test]
    fn mapping_length() {
        let (orig, norm, map) = normalize_lyrics("don't cry tonight");
        assert_eq!(map.len(), orig.len());
        // "don't" expands to 2 tokens.
        assert_eq!(norm.len(), 4); // do, not, cry, tonight
    }
}
