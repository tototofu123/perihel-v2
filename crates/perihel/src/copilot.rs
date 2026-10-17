//! Copilot loop — grammar-constrained tool calling with up to 3 tool rounds.
//!
//! Round: llama.cpp samples `{"tool":...,"args":{...}}` under the GBNF grammar
//! (invalid calls are physically unproducible), we parse + execute it, then feed
//! the observation back. `reply.text` ends the loop with the user-facing answer.
//! Any parse failure degrades to `reply.text` carrying the raw output — a bad
//! model day costs one weird answer, never a crash or a lost capture.

use crate::llm::LlamaSidecar;
use crate::tools::{self, ToolCtx};
use crate::vision::VisionSidecar;
use perihel_core::{ResourceGuard, ToolCall};
use rusqlite::Connection;
use serde_json::json;
use std::sync::{Arc, Mutex};

/// GBNF grammar shipped with the repo — embedded at build time.
pub const TOOLS_GRAMMAR: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../grammar/perihel-tools.gbnf"));

pub struct CopilotOutcome {
    pub answer: String,
    pub calls: Vec<(String, String)>, // (tool, result preview) for the log UI
}

#[allow(clippy::too_many_arguments)]
pub fn run_copilot(
    llm: &Arc<Mutex<LlamaSidecar>>,
    vision: &Arc<Mutex<VisionSidecar>>,
    conn: &Arc<Mutex<Connection>>,
    data_dir: std::path::PathBuf,
    guard: &ResourceGuard,
    user_input: &str,
    tts_enabled: bool,
    tts_voice: String,
) -> CopilotOutcome {
    let mut messages = vec![
        json!({"role": "system", "content": perihel_core::system_prompt()}),
        json!({"role": "user", "content": user_input}),
    ];
    let mut calls: Vec<(String, String)> = Vec::new();
    let mut final_answer: Option<String> = None;

    for _round in 0..3 {
        let raw = {
            let mut sidecar = llm.lock().unwrap();
            match sidecar.ensure_running(guard).and_then(|_| sidecar.chat(&messages, Some(TOOLS_GRAMMAR), 350)) {
                Ok(r) => r,
                Err(e) => {
                    return CopilotOutcome {
                        answer: format!("Copilot unavailable: {e}"),
                        calls,
                    }
                }
            }
        };

        let call = ToolCall::parse_call(&raw);
        if let ToolCall::Reply { text } = call {
            final_answer = Some(text);
            break;
        }

        let result = {
            let mut ctx = ToolCtx {
                conn,
                data_dir: data_dir.clone(),
                tts_enabled,
                tts_voice: tts_voice.clone(),
                vision,
                guard,
            };
            match tools::execute(&mut ctx, call.clone()) {
                Ok(r) => r,
                Err(e) => format!("tool error: {e}"),
            }
        };
        log::info!("tool {} → {}", call.name(), result.chars().take(120).collect::<String>());
        calls.push((call.name().to_string(), result.chars().take(160).collect()));

        messages.push(json!({"role": "assistant", "content": raw}));
        messages.push(json!({"role": "user", "content": format!("tool result:\n{result}\n\nReply to the user now, or make another tool call.")}));
    }

    let answer = final_answer.unwrap_or_else(|| {
        // loop exhausted without a reply — try one unconstrained summary
        let mut sidecar = llm.lock().unwrap();
        sidecar
            .chat(
                &[
                    json!({"role": "system", "content": "Summarize the tool results for the user in one short paragraph."}),
                    json!({"role": "user", "content": format!("{:?}", calls)}),
                ],
                None,
                200,
            )
            .unwrap_or_else(|e| format!("Tools ran ({}) but the summary failed: {e}", calls.len()))
    });

    log_tool_round(conn, user_input, &calls, &answer);
    CopilotOutcome { answer, calls }
}

fn log_tool_round(conn: &Arc<Mutex<Connection>>, input: &str, calls: &[(String, String)], answer: &str) {
    if let Ok(c) = conn.lock() {
        let _ = perihel_core::db::log_tool_call(
            &c,
            input,
            &calls.iter().map(|(t, r)| format!("{t}: {r}")).collect::<Vec<_>>().join(" | "),
            answer,
        );
    }
}
