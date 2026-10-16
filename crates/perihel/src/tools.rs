//! Tool dispatcher — executes a `perihel.tools/v1` ToolCall against local state.
//! Every result is a plain string handed back to the model as a tool observation.

use crate::agents;
use crate::tts;
use crate::vision::VisionSidecar;
use anyhow::Result;
use perihel_core::{ResourceGuard, ToolCall};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub struct ToolCtx<'a> {
    pub conn: &'a Arc<Mutex<Connection>>,
    pub data_dir: PathBuf,
    pub tts_enabled: bool,
    pub tts_voice: String,
    pub vision: &'a Arc<Mutex<VisionSidecar>>,
    pub guard: &'a ResourceGuard,
}

pub fn execute(ctx: &mut ToolCtx, call: ToolCall) -> Result<String> {
    match call {
        ToolCall::Capture { text } => {
            let id = {
                let conn = ctx.conn.lock().unwrap();
                perihel_core::db::insert_capture(&conn, &text, "text")?
            };
            Ok(format!("Saved capture #{id}: {}", preview(&text, 80)))
        }

        ToolCall::ReminderSchedule { text, at } => {
            let due = next_occurrence(&at)?;
            let id = {
                let conn = ctx.conn.lock().unwrap();
                perihel_core::db::insert_reminder(&conn, &text, &due)?
            };
            Ok(format!("Reminder #{id} set for {due}: {text}"))
        }

        ToolCall::ReminderList {} => {
            let conn = ctx.conn.lock().unwrap();
            let reminders = perihel_core::db::get_pending_reminders(&conn)?;
            if reminders.is_empty() {
                Ok("No pending reminders.".into())
            } else {
                Ok(reminders
                    .iter()
                    .map(|r| format!("#{} at {} — {}", r.id, r.due_at, r.text))
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
        }

        ToolCall::ReminderComplete { id } => {
            let conn = ctx.conn.lock().unwrap();
            perihel_core::db::complete_reminder(&conn, id as i64)?;
            Ok(format!("Reminder #{id} marked done."))
        }

        ToolCall::SearchCaptures { query } => {
            let conn = ctx.conn.lock().unwrap();
            let hits = perihel_core::db::search_captures(&conn, &query, 5)?;
            if hits.is_empty() {
                Ok(format!("No captures matching '{query}'."))
            } else {
                Ok(hits
                    .iter()
                    .map(|c| format!("#{} ({}) {}: {}", c.id, c.created_at, c.status, preview(&c.content, 120)))
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
        }

        ToolCall::Speak { text } => {
            if ctx.tts_enabled {
                tts::speak(&text, &ctx.tts_voice)?;
                Ok(format!("Spoke aloud: {}", preview(&text, 60)))
            } else {
                Ok("TTS is disabled in settings.".into())
            }
        }

        ToolCall::ScreenshotTake {} => {
            match take_screenshot(&ctx.data_dir) {
                Some(p) => Ok(format!("Screenshot saved to {p}")),
                None => Err(anyhow::anyhow!("screenshot capture failed")),
            }
        }

        ToolCall::VisionDescribe { query } => {
            let q = if query.trim().is_empty() { "Describe what is on this screen." } else { &query };
            let mut vision = ctx.vision.lock().unwrap();
            match vision.describe_latest(&ctx.data_dir, q, ctx.guard) {
                Ok(desc) => Ok(desc),
                Err(e) => Ok(format!("Vision unavailable: {e}")),
            }
        }

        ToolCall::AgentDispatch { prompt } => dispatch_agent(&prompt),

        ToolCall::Reply { text } => Ok(text),
    }
}

fn preview(s: &str, n: usize) -> String {
    let t = s.trim();
    if t.chars().count() > n {
        format!("{}…", t.chars().take(n).collect::<String>())
    } else {
        t.to_string()
    }
}

/// HH:MM local → RFC3339 due time (today if still ahead, else tomorrow).
fn next_occurrence(hhmm: &str) -> Result<String> {
    let b = hhmm.as_bytes();
    // "HH:MM" is 5 bytes; anything shorter isn't zero-padded.
    if b.len() != 5 || b[2] != b':' || !b[..2].iter().all(|c| c.is_ascii_digit()) || !b[3..].iter().all(|c| c.is_ascii_digit()) {
        anyhow::bail!("invalid time {hhmm:?}");
    }
    let hour: u32 = hhmm[..2].parse()?;
    let min: u32 = hhmm[3..].parse()?;
    if hour > 23 || min > 59 {
        anyhow::bail!("invalid time {hhmm:?}");
    }
    let now = chrono::Local::now();
    let today_due = now.date_naive().and_hms_opt(hour, min, 0).unwrap();
    let tomorrow_due = (now.date_naive() + chrono::Duration::days(1))
        .and_hms_opt(hour, min, 0)
        .unwrap();
    let due = if today_due > now.naive_local() {
        today_due.and_local_timezone(chrono::Local).single()
    } else {
        tomorrow_due.and_local_timezone(chrono::Local).single()
    };
    Ok(due.unwrap_or(now).to_rfc3339())
}

pub fn take_screenshot(data_dir: &Path) -> Option<String> {
    let dir = data_dir.join("screenshots");
    std::fs::create_dir_all(&dir).ok()?;
    let stamp = chrono::Local::now().format("%Y%m%d_%H%M%S_%3f");
    let path = dir.join(format!("{stamp}.png"));
    let monitors = xcap::Monitor::all().ok()?;
    let image = monitors.first()?.capture_image().ok()?;
    image.save(&path).ok()?;
    Some(path.to_string_lossy().to_string())
}

fn dispatch_agent(prompt: &str) -> Result<String> {
    let agents = agents::detect_agents();
    let pick = agents
        .iter()
        .find(|a| a.detected && a.id != "vox" && a.path.is_some())
        .ok_or_else(|| anyhow::anyhow!("no coding agent CLI detected"))?;
    let bin = pick.path.clone().unwrap();

    let mut cmd = std::process::Command::new(bin);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.arg("--prompt").arg(prompt)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());

    // bounded: agent dispatches are capped at 5 minutes
    let child = cmd.spawn()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
    let mut stdout = String::new();
    let mut out = child.stdout.unwrap();
    use std::io::Read;
    loop {
        let mut buf = [0u8; 4096];
        match out.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => stdout.push_str(&String::from_utf8_lossy(&buf[..n])),
            Err(_) => break,
        }
        if std::time::Instant::now() > deadline {
            break;
        }
        if stdout.len() > 20_000 {
            break;
        }
    }
    let trimmed = stdout.trim().to_string();
    Ok(if trimmed.is_empty() { format!("Agent {} returned no output.", pick.name) } else { preview(&trimmed, 1500) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_occurrence_validates() {
        assert!(next_occurrence("25:00").is_err(), "hour > 23");
        assert!(next_occurrence("15:60").is_err(), "minute > 59");
        assert!(next_occurrence("9:30").is_err(), "must be zero-padded HH:MM");
        assert!(next_occurrence("15:0").is_err(), "must be zero-padded HH:MM");
        assert!(next_occurrence("1a:00").is_err(), "non-digit hour");
        assert!(next_occurrence("15:0x").is_err(), "non-digit minute");
        assert!(next_occurrence("1500").is_err(), "missing colon");

        // Boundary values that must be accepted.
        assert!(next_occurrence("00:00").is_ok());
        assert!(next_occurrence("23:59").is_ok());
        assert!(next_occurrence("15:00").is_ok());
    }
}
