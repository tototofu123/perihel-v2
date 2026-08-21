//! TextFormatter — RuleBased + Dictionary correction (mirrors murmur TextFormatter.swift)

pub mod dictionary;

use regex::Regex;

pub trait TextFormatter {
    fn format(&self, raw: &str) -> String;
}

pub struct RuleBasedFormatter {
    filler_re: Regex,
}

impl RuleBasedFormatter {
    pub fn new() -> Self {
        // strip fillers whole-word, case-insensitive
        let re = Regex::new(r"(?i)\b(um+|uh+|er+|ah+|like|you know)\b[,\s]*").unwrap();
        Self { filler_re: re }
    }
}

impl TextFormatter for RuleBasedFormatter {
    fn format(&self, raw: &str) -> String {
        let mut s = self.filler_re.replace_all(raw, "").to_string();
        // fix spacing
        s = Regex::new(r"\s+").unwrap().replace_all(&s, " ").trim().to_string();
        // capitalise sentences + terminal punct
        if !s.is_empty() {
            let mut chars: Vec<char> = s.chars().collect();
            chars[0] = chars[0].to_ascii_uppercase();
            s = chars.into_iter().collect();
            if !matches!(s.chars().last(), Some('.' | '!' | '?')) {
                s.push('.');
            }
        }
        s
    }
}
