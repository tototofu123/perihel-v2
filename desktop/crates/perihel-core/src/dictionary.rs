//! DictionaryCorrector — whole-word, longest-first, optional [\s-]* between parts
//! Contract: shared/dictionary-test-vectors.json (19 vectors from murmur)

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use unicode_normalization::UnicodeNormalization;

fn enabled_by_default() -> bool {
    true
}

/// A rewrite rule: when the STT hears `hear`, write `write` instead.
/// `is_enabled` is contract rule 5 — a disabled entry must change nothing.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Correction {
    pub hear: String,
    pub write: String,
    #[serde(default = "enabled_by_default")]
    pub is_enabled: bool,
}

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
        let path = crate::paths::app_config_dir().join("dictionary.json");
        let dict = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self::from_dict(dict)
    }

    pub fn from_dict(dict: Dictionary) -> Self {
        let mut corrections: Vec<Correction> =
            dict.corrections.iter().filter(|c| c.is_enabled).cloned().collect();
        corrections.sort_by_key(|c| std::cmp::Reverse(c.hear.len())); // longest first
        let mut regexes = Vec::new();
        for c in corrections {
            let pat = build_pattern(&c.hear);
            match Regex::new(&format!(r"(?i){pat}")) {
                Ok(re) => regexes.push((re, c.write.clone())),
                // Never fail silently — a dropped entry is a silently broken feature.
                Err(e) => log::error!("dictionary entry {:?} is invalid and was skipped: {e}", c.hear),
            }
        }
        Self { dict, regexes }
    }

    pub fn correct(&self, text: &str) -> String {
        self.correct_with_counts(text).0
    }

    /// Correct `text` and report how many times each replacement fired.
    ///
    /// The counts are the contract's `expectedCorrections`, so the shared
    /// vectors can assert both the output text and the number of hits.
    pub fn correct_with_counts(&self, text: &str) -> (String, Vec<(String, usize)>) {
        // NFC normalize both sides (macOS hands back NFD from several APIs)
        let mut out: String = text.nfc().collect();
        let mut counts: Vec<(String, usize)> = Vec::new();
        for (re, rep) in &self.regexes {
            let hits = re.find_iter(&out).count();
            if hits == 0 {
                continue;
            }
            match counts.iter_mut().find(|(w, _)| w == rep) {
                Some(slot) => slot.1 += hits,
                None => counts.push((rep.clone(), hits)),
            }
            // Closure form, so a `$` inside the replacement is literal text
            // rather than a capture-group reference.
            out = re.replace_all(&out, |_: &regex::Captures<'_>| rep.as_str()).to_string();
        }
        (out, counts)
    }

    pub fn bias_words(&self) -> &[String] {
        // keep short — long context makes parakeet hallucinate (murmur docs)
        &self.dict.words[..self.dict.words.len().min(20)]
    }
}

fn build_pattern(hear: &str) -> String {
    // The trigger is what the STT *hears*, not the canonical spelling: a
    // "cloud code" -> "Claude Code" entry becomes `cloud[\s\-]*code`, which is
    // what catches the glued and hyphenated forms ("CloudCode", "Cloud-Code")
    // as well as the plain one.
    let parts: Vec<String> = hear.split_whitespace().map(regex::escape).collect();
    let core = parts.join(r"[\s\-]*");

    // Whole-word matching (contract rule 1) via \b, because the regex crate
    // has no look-around. \b is only meaningful next to a word character, so it
    // is added per side — that keeps triggers like "c++" or ".net" working,
    // where a boundary would otherwise never match.
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let lead = if core.starts_with(is_word) { r"\b" } else { "" };
    let trail = if core.ends_with(is_word) { r"\b" } else { "" };
    format!("{lead}{core}{trail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `hear` is the mis-hearing, `write` the canonical spelling — see
    /// shared/dictionary-test-vectors.json. Triggering on "cloud code" is what
    /// makes the glued/hyphenated variants match.
    fn claude_dict() -> DictionaryCorrector {
        DictionaryCorrector::from_dict(Dictionary {
            words: vec![],
            corrections: vec![Correction { hear: "cloud code".into(), write: "Claude Code".into(), is_enabled: true }],
        })
    }

    #[test]
    fn claude_code_variants() {
        let c = claude_dict();
        assert_eq!(c.correct("I use cloud code"), "I use Claude Code");
        assert_eq!(c.correct("I use CloudCode"), "I use Claude Code");
        assert_eq!(c.correct("I use Cloud-Code"), "I use Claude Code");
        assert_eq!(c.correct("I use Cloud   Code"), "I use Claude Code");
    }

    #[test]
    fn is_case_insensitive_and_whole_word_only() {
        let c = claude_dict();
        assert_eq!(c.correct("CLOUD CODE"), "Claude Code");
        // Rule 1: must never bite into a longer word.
        assert_eq!(c.correct("I use Cloudflare"), "I use Cloudflare");
        assert_eq!(c.correct("I use CloudCodes"), "I use CloudCodes");
    }

    /// Rule 3: a longer trigger wins over a shorter overlapping one.
    #[test]
    fn longest_trigger_wins() {
        let c = DictionaryCorrector::from_dict(Dictionary {
            words: vec![],
            corrections: vec![
                Correction { hear: "cloud".into(), write: "CLOUD".into(), is_enabled: true },
                Correction { hear: "cloud code".into(), write: "Claude Code".into(), is_enabled: true },
            ],
        });
        assert_eq!(c.correct("I use cloud code"), "I use Claude Code");
        assert_eq!(c.correct("a cloud"), "a CLOUD");
    }

    /// Contract rule 5: a disabled entry must be completely inert.
    #[test]
    fn disabled_entry_changes_nothing() {
        let c = DictionaryCorrector::from_dict(Dictionary {
            words: vec![],
            corrections: vec![Correction {
                hear: "cloud code".into(),
                write: "Claude Code".into(),
                is_enabled: false,
            }],
        });
        assert_eq!(c.correct("I use cloud code every day."), "I use cloud code every day.");
        assert_eq!(c.correct_with_counts("cloud code").1, vec![]);
    }

    /// `correct_with_counts` reports per-replacement hit counts, which is what
    /// the shared vectors assert.
    #[test]
    fn reports_replacement_counts() {
        let c = DictionaryCorrector::from_dict(Dictionary {
            words: vec![],
            corrections: vec![Correction {
                hear: "clawed code".into(),
                write: "Claude Code".into(),
                is_enabled: true,
            }],
        });
        let (out, counts) = c.correct_with_counts("clawed code, then clawed code again, and clawed code.");
        assert_eq!(out, "Claude Code, then Claude Code again, and Claude Code.");
        assert_eq!(counts, vec![("Claude Code".to_string(), 3)]);
    }
}
