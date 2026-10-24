//! RuleBasedFormatter — deterministic dictation cleanup (no LLM).
//! Whole-word filler strip, whitespace collapse, sentence caps, terminal punct.

use regex::Regex;

pub trait TextFormatter {
    fn format(&self, raw: &str) -> String;
}

pub struct RuleBasedFormatter {
    filler_re: Regex,
    ws_re: Regex,
}

impl Default for RuleBasedFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl RuleBasedFormatter {
    pub fn new() -> Self {
        // strip fillers whole-word, case-insensitive
        let filler_re = Regex::new(r"(?i)\b(um+|uh+|er+|ah+|like|you know)\b[,\s]*").unwrap();
        let ws_re = Regex::new(r"\s+").unwrap();
        Self { filler_re, ws_re }
    }
}

impl TextFormatter for RuleBasedFormatter {
    fn format(&self, raw: &str) -> String {
        let mut s = self.filler_re.replace_all(raw, "").to_string();
        s = self.ws_re.replace_all(&s, " ").trim().to_string();
        if !s.is_empty() {
            let mut chars: Vec<char> = s.chars().collect();
            chars[0] = chars[0].to_ascii_uppercase();
            s = chars.into_iter().collect();
            if !matches!(s.chars().last(), Some('.') | Some('!') | Some('?')) {
                s.push('.');
            }
        }
        s
    }
}

/// One-shot: rule-based format + dictionary correction, the way dictation output
/// is always cleaned before injection or capture storage.
pub fn format_text(raw: &str) -> String {
    let formatter = RuleBasedFormatter::new();
    let corrector = crate::dictionary::DictionaryCorrector::load_or_default();
    corrector.correct(&formatter.format(raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_fillers_and_caps() {
        let f = RuleBasedFormatter::new();
        assert_eq!(f.format("um so I guess like we should ship it"), "So I guess we should ship it.");
    }

    #[test]
    fn keeps_terminal_punct() {
        let f = RuleBasedFormatter::new();
        assert_eq!(f.format("already done!"), "Already done!");
    }
}
