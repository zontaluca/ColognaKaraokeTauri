use std::collections::HashMap;
use std::path::Path;

use crate::AlignError;

const DEFAULT_PAD: &str = "<pad>";
const DEFAULT_UNK: &str = "<unk>";
const DEFAULT_DELIMITER: &str = "|";

/// Char-level vocabulary used by wav2vec2 CTC heads. Loaded from `vocab.json`.
///
/// HF convention: `<pad>` doubles as the CTC blank id. Word boundaries are
/// emitted as `|`. Unknown chars map to `<unk>`.
#[derive(Debug, Clone)]
pub struct Vocab {
    pub char_to_id: HashMap<String, u32>,
    pub id_to_char: HashMap<u32, String>,
    pub pad_id: u32,
    pub unk_id: u32,
    pub delimiter_id: u32,
    pub vocab_size: u32,
}

impl Vocab {
    pub fn from_json(path: &Path) -> Result<Self, AlignError> {
        let bytes = std::fs::read(path).map_err(|e| {
            AlignError::Config(format!("read vocab {}: {}", path.display(), e))
        })?;
        Self::from_bytes(&bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, AlignError> {
        let raw: HashMap<String, u32> = serde_json::from_slice(bytes)
            .map_err(|e| AlignError::Config(format!("parse vocab.json: {}", e)))?;
        if raw.is_empty() {
            return Err(AlignError::Config("vocab.json is empty".into()));
        }

        let mut id_to_char: HashMap<u32, String> = HashMap::with_capacity(raw.len());
        for (k, &v) in &raw {
            id_to_char.insert(v, k.clone());
        }

        let pad_id = *raw
            .get(DEFAULT_PAD)
            .ok_or_else(|| AlignError::Config("vocab.json missing <pad>".into()))?;
        let unk_id = *raw
            .get(DEFAULT_UNK)
            .ok_or_else(|| AlignError::Config("vocab.json missing <unk>".into()))?;
        let delimiter_id = *raw
            .get(DEFAULT_DELIMITER)
            .ok_or_else(|| AlignError::Config("vocab.json missing word delimiter '|'".into()))?;

        let vocab_size = raw.values().copied().max().map(|m| m + 1).unwrap_or(0);

        Ok(Self {
            char_to_id: raw,
            id_to_char,
            pad_id,
            unk_id,
            delimiter_id,
            vocab_size,
        })
    }

    /// Map a single character to an id, falling back to `<unk>`.
    pub fn id_for_char(&self, c: char) -> u32 {
        let mut buf = [0u8; 4];
        let s = c.encode_utf8(&mut buf);
        self.char_to_id.get(s).copied().unwrap_or(self.unk_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_vocab() {
        let json = br#"{"<pad>":0,"<s>":1,"</s>":2,"<unk>":3,"|":4,"a":5,"b":6}"#;
        let v = Vocab::from_bytes(json).unwrap();
        assert_eq!(v.pad_id, 0);
        assert_eq!(v.unk_id, 3);
        assert_eq!(v.delimiter_id, 4);
        assert_eq!(v.vocab_size, 7);
        assert_eq!(v.id_for_char('a'), 5);
        assert_eq!(v.id_for_char('z'), v.unk_id);
    }
}
