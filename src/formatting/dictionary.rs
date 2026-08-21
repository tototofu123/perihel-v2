//! DictionaryCorrector — whole-word, longest-first, optional [\s-]* between parts
//! Contract: shared/dictionary-test-vectors.json (19 vectors from murmur)

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::PathBuf};
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Correction { pub hear: String, pub write: String }

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Dictionary {
    pub words: Vec<String>, // bias words (passed to engine, max 20)
    pub corrections: Vec<Correction>,
}

pub struct DictionaryCorrector {
    dict: Dictionary,
    regexes: Vec<(Regex, String)>,
}

impl DictionaryCorrector {
    pub fn load_or_default() -> Self {
        let path = crate::support::paths::dictionary_path();
        let dict = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self::from_dict(dict)
    }

    pub fn from_dict(dict: Dictionary) -> Self {
        let mut corrections = dict.corrections.clone();
        corrections.sort_by(|a,b| b.hear.len().cmp(&a.hear.len())); // longest first
        let mut regexes = Vec::new();
        for c in corrections {
            let pat = build_pattern(&c.hear);
            if let Ok(re) = Regex::new(&format!(r"(?i)(?<!\w){}(?!\w)", pat)) {
                regexes.push((re, c.write.clone()));
            }
        }
        Self { dict, regexes }
    }

    pub fn correct(&self, text: &str) -> String {
        // NFC normalize both sides (murmur handles NFD vs NFC)
        let norm: String = text.nfc().collect();
        let mut out = norm;
        for (re, rep) in &self.regexes {
            out = re.replace_all(&out, rep.as_str()).to_string();
        }
        out
    }

    pub fn bias_words(&self) -> &[String] {
        // keep short — long context makes parakeet hallucinate (murmur docs)
        &self.dict.words[..self.dict.words.len().min(20)]
    }
}

fn build_pattern(hear: &str) -> String {
    // "Claude Code" -> "Claude[\s\-]*Code" with escaping, to catch CloudCode/Cloud-Code
    let parts: Vec<String> = hear.split_whitespace().map(|p| regex::escape(p)).collect();
    parts.join(r"[\s\-]*")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn claude_code_variants() {
        let d = Dictionary { words: vec![], corrections: vec![Correction{ hear: "Claude Code".into(), write: "Claude Code".into()}]};
        let c = DictionaryCorrector::from_dict(d);
        assert_eq!(c.correct("I use CloudCode"), "I use Claude Code");
        assert_eq!(c.correct("I use Cloud-Code"), "I use Claude Code");
        assert_ne!(c.correct("Cloudflare"), "Claude Code"); // must NOT touch
    }
}
