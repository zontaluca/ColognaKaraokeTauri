use crate::vocab::Vocab;

/// Output of [`lyrics_to_target_ids`]. Each entry corresponds to one
/// whitespace-separated word in the original lyrics.
#[derive(Debug, Clone)]
pub struct WordTokens {
    /// Original surface form (case + punctuation preserved). Used for display.
    pub word: String,
    /// Inclusive `[start, end)` range into the flat CTC target id stream.
    /// The range covers the char ids for this word; delimiter ids fall outside.
    pub token_range: std::ops::Range<usize>,
}

#[derive(Debug, Clone)]
pub struct TargetSequence {
    pub ids: Vec<u32>,
    pub words: Vec<WordTokens>,
}

/// Build the CTC target id sequence from `lyrics`. Words are separated by
/// `|` (vocab.delimiter_id). Each word's chars are mapped through `vocab`,
/// with unknown chars folded to `<unk>` (no skipping, so frame counts stay
/// aligned with display words).
///
/// Lowercases all input — XLSR-53 char vocabs are lowercase. Skips empty words
/// after normalisation. The first and last delimiters are omitted so the
/// extended-target sequence in CTC doesn't double up on blanks.
pub fn lyrics_to_target_ids(lyrics: &str, vocab: &Vocab) -> TargetSequence {
    let mut ids: Vec<u32> = Vec::new();
    let mut words: Vec<WordTokens> = Vec::new();

    for raw in lyrics.split_whitespace() {
        let surface = raw.to_string();
        let normalised: String = raw
            .chars()
            .filter(|c| c.is_alphabetic() || *c == '\'' || *c == '-')
            .flat_map(|c| c.to_lowercase())
            .collect();
        if normalised.is_empty() {
            continue;
        }

        if !words.is_empty() {
            ids.push(vocab.delimiter_id);
        }

        let start = ids.len();
        for ch in normalised.chars() {
            ids.push(vocab.id_for_char(ch));
        }
        let end = ids.len();

        words.push(WordTokens {
            word: surface,
            token_range: start..end,
        });
    }

    TargetSequence { ids, words }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_vocab() -> Vocab {
        let json = br#"{"<pad>":0,"<s>":1,"</s>":2,"<unk>":3,"|":4,"a":5,"b":6,"c":7,"i":8,"o":9}"#;
        Vocab::from_bytes(json).unwrap()
    }

    #[test]
    fn builds_target_with_delimiters() {
        let vocab = fake_vocab();
        let seq = lyrics_to_target_ids("Ab ciao!", &vocab);
        // a b | c i a o
        assert_eq!(seq.ids, vec![5, 6, 4, 7, 8, 5, 9]);
        assert_eq!(seq.words.len(), 2);
        assert_eq!(seq.words[0].word, "Ab");
        assert_eq!(seq.words[0].token_range, 0..2);
        assert_eq!(seq.words[1].word, "ciao!");
        assert_eq!(seq.words[1].token_range, 3..7);
    }

    #[test]
    fn maps_unknown_chars_to_unk() {
        let vocab = fake_vocab();
        let seq = lyrics_to_target_ids("xyz", &vocab);
        assert_eq!(seq.ids, vec![vocab.unk_id, vocab.unk_id, vocab.unk_id]);
    }
}
