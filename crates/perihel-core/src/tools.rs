//! perihel.tools/v1 — the custom tool-calling contract.
//!
//! One enum, one grammar, one prompt — shared by:
//! - the runtime dispatcher (crates/perihel/src/tools.rs executes these),
//! - llama.cpp (`grammar/perihel-tools.gbnf` constrains generation to this set),
//! - the fine-tuning dataset (training/build_dataset.py emits exactly this shape).
//!
//! The model output is always `{"tool":"...","args":{...}}`. If the model (or a
//! plain non-fine-tuned model) emits anything else, `parse_call` falls back to
//! `reply.text` with the raw output — a capture is never lost to a parse error.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "tool", content = "args")]
pub enum ToolCall {
    /// Store a fleeting thought / note / link into the capture inbox.
    #[serde(rename = "capture.create")]
    Capture { text: String },

    /// Schedule a reminder. `at` is local `HH:MM` (next occurrence — today if still
    /// in the future, otherwise tomorrow).
    #[serde(rename = "reminder.schedule")]
    ReminderSchedule { text: String, at: String },

    /// List pending reminders.
    #[serde(rename = "reminder.list")]
    ReminderList {},

    /// Mark reminder done by id.
    #[serde(rename = "reminder.complete")]
    ReminderComplete { id: u64 },

    /// Full-text search over the user's capture history (local SQLite).
    #[serde(rename = "search.captures")]
    SearchCaptures { query: String },

    /// Speak text aloud (TTS).
    #[serde(rename = "tts.speak")]
    Speak { text: String },

    /// Capture the primary monitor to a PNG.
    #[serde(rename = "screenshot.take")]
    ScreenshotTake {},

    /// Describe the most recent screenshot (vision model sidecar).
    #[serde(rename = "vision.describe")]
    VisionDescribe { query: String },

    /// Hand a coding/system task to a detected external agent CLI.
    #[serde(rename = "agent.dispatch")]
    AgentDispatch { prompt: String },

    /// Plain conversational reply — the safe fallback for everything.
    #[serde(rename = "reply.text")]
    Reply { text: String },
}

impl ToolCall {
    /// Tool names in grammar order — used for prompt building and dataset stats.
    pub const NAMES: &'static [&'static str] = &[
        "capture.create",
        "reminder.schedule",
        "reminder.list",
        "reminder.complete",
        "search.captures",
        "tts.speak",
        "screenshot.take",
        "vision.describe",
        "agent.dispatch",
        "reply.text",
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Capture { .. } => "capture.create",
            Self::ReminderSchedule { .. } => "reminder.schedule",
            Self::ReminderList {} => "reminder.list",
            Self::ReminderComplete { .. } => "reminder.complete",
            Self::SearchCaptures { .. } => "search.captures",
            Self::Speak { .. } => "tts.speak",
            Self::ScreenshotTake {} => "screenshot.take",
            Self::VisionDescribe { .. } => "vision.describe",
            Self::AgentDispatch { .. } => "agent.dispatch",
            Self::Reply { .. } => "reply.text",
        }
    }

    /// Parse raw model output into a call. Never fails: anything unparseable
    /// becomes `reply.text` carrying the raw string.
    pub fn parse_call(raw: &str) -> Self {
        let trimmed = raw.trim();
        // strip a possible <answer> fence some templates add
        let cleaned = trimmed
            .strip_prefix("<answer>")
            .or_else(|| trimmed.strip_prefix("```json"))
            .unwrap_or(trimmed);
        let cleaned = cleaned
            .strip_suffix("</answer>")
            .or_else(|| cleaned.strip_suffix("```"))
            .unwrap_or(cleaned)
            .trim();
        match serde_json::from_str::<ToolCall>(cleaned) {
            Ok(call) => {
                // Validate the reminder time shape even though the grammar usually
                // guarantees it. Must be zero-padded HH:MM with real clock values.
                if let ToolCall::ReminderSchedule { at, .. } = &call {
                    let b = at.as_bytes();
                    let ok = b.len() == 5
                        && b[2] == b':'
                        && b[..2].iter().all(u8::is_ascii_digit)
                        && b[3..].iter().all(u8::is_ascii_digit)
                        && at[..2].parse::<u32>().is_ok_and(|h| h <= 23)
                        && at[3..].parse::<u32>().is_ok_and(|m| m <= 59);
                    if !ok {
                        return Self::Reply { text: cleaned.to_string() };
                    }
                }
                call
            }
            Err(_) => Self::Reply { text: cleaned.to_string() },
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }
}

/// The system prompt used at runtime AND in the fine-tune dataset, so the
/// fine-tuned model sees an identical distribution at train and inference time.
pub fn system_prompt() -> String {
    r#"You are Perihel, a local desktop companion. You turn what the user says into exactly ONE tool call, output as a single JSON object and nothing else.

Tools:
- capture.create {"text": str} — save a thought, note, or link for later resurfacing. Use when the user mentions an idea, todo-ish note, or "remember this".
- reminder.schedule {"text": str, "at": "HH:MM"} — set a time-based reminder. Use for "remind me at 15:00", "in 2 hours" (compute the clock time), "tomorrow morning".
- reminder.list {} — list pending reminders.
- reminder.complete {"id": int} — mark a reminder done.
- search.captures {"query": str} — search past captures/notes.
- tts.speak {"text": str} — say something out loud.
- screenshot.take {} — grab the screen.
- vision.describe {"query": str} — describe the latest screenshot.
- agent.dispatch {"prompt": str} — delegate a coding/system task to a coding agent.
- reply.text {"text": str} — answer conversationally. Default when nothing else fits.

Rules:
- Output ONLY the JSON object: {"tool":"...","args":{...}}
- Exactly one call per turn. Never wrap in prose or code fences.
- Prefer a real tool over reply.text whenever one fits.
- For reminder.schedule compute "at" as local HH:MM (24h).
- Keep text concise; strip filler words."#
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_calls() {
        let c = ToolCall::parse_call(r#"{"tool":"reminder.schedule","args":{"text":"ask Prof. Chan","at":"15:00"}}"#);
        assert_eq!(c.name(), "reminder.schedule");
        assert_eq!(c.to_json(), r#"{"tool":"reminder.schedule","args":{"text":"ask Prof. Chan","at":"15:00"}}"#);
    }

    #[test]
    fn falls_back_to_reply_on_garbage() {
        let c = ToolCall::parse_call("Sure! I'll remind you later.");
        assert_eq!(c.name(), "reply.text");
        match c {
            ToolCall::Reply { text } => assert_eq!(text, "Sure! I'll remind you later."),
            _ => unreachable!(),
        }
    }

    #[test]
    fn falls_back_on_bad_time() {
        let c = ToolCall::parse_call(r#"{"tool":"reminder.schedule","args":{"text":"x","at":"25:99"}}"#);
        assert_eq!(c.name(), "reply.text");
    }

    #[test]
    fn strips_answer_fences() {
        let c = ToolCall::parse_call("<answer>{\"tool\":\"reminder.list\",\"args\":{}}</answer>");
        assert_eq!(c.name(), "reminder.list");
    }

    #[test]
    fn rejects_unpadded_and_out_of_range_times() {
        for bad in [
            r#"{"tool":"reminder.schedule","args":{"text":"x","at":"9:30"}}"#,
            r#"{"tool":"reminder.schedule","args":{"text":"x","at":"25:00"}}"#,
            r#"{"tool":"reminder.schedule","args":{"text":"x","at":"15:60"}}"#,
            r#"{"tool":"reminder.schedule","args":{"text":"x","at":"1500"}}"#,
            r#"{"tool":"reminder.schedule","args":{"text":"x","at":""}}"#,
        ] {
            assert_eq!(
                ToolCall::parse_call(bad).name(),
                "reply.text",
                "should have rejected {bad}"
            );
        }
    }

    #[test]
    fn accepts_boundary_times() {
        for good in ["00:00", "09:05", "15:00", "23:59"] {
            let raw = format!(r#"{{"tool":"reminder.schedule","args":{{"text":"x","at":"{good}"}}}}"#);
            assert_eq!(ToolCall::parse_call(&raw).name(), "reminder.schedule", "{good}");
        }
    }

    /// The GBNF grammar always emits an `"args"` key, even for argument-less
    /// tools, so every variant must survive parse -> serialize unchanged.
    #[test]
    fn round_trips_every_tool_through_the_grammar_shape() {
        let cases = [
            ("capture.create", r#"{"tool":"capture.create","args":{"text":"hi"}}"#),
            ("reminder.schedule", r#"{"tool":"reminder.schedule","args":{"text":"ask Prof. Chan","at":"15:00"}}"#),
            ("reminder.list", r#"{"tool":"reminder.list","args":{}}"#),
            ("reminder.complete", r#"{"tool":"reminder.complete","args":{"id":7}}"#),
            ("search.captures", r#"{"tool":"search.captures","args":{"query":"notes"}}"#),
            ("tts.speak", r#"{"tool":"tts.speak","args":{"text":"hello"}}"#),
            ("screenshot.take", r#"{"tool":"screenshot.take","args":{}}"#),
            ("vision.describe", r#"{"tool":"vision.describe","args":{"query":"what is this"}}"#),
            ("agent.dispatch", r#"{"tool":"agent.dispatch","args":{"prompt":"fix the build"}}"#),
            ("reply.text", r#"{"tool":"reply.text","args":{"text":"ok"}}"#),
        ];
        assert_eq!(cases.len(), ToolCall::NAMES.len(), "a tool is missing from the round-trip table");
        for (want, raw) in cases {
            let call = ToolCall::parse_call(raw);
            assert_eq!(call.name(), want, "wrong tool for {raw}");
            assert_eq!(call.to_json(), raw, "round-trip mismatch for {raw}");
        }
    }
}
