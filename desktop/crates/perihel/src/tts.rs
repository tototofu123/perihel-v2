//! TTS — Windows SAPI via PowerShell. Zero dependencies, zero model RAM: the OS
//! ships the voices. Used by `tts.speak` tool calls and reminder announcements.

use anyhow::{Context, Result};
use std::process::Command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Speak text asynchronously (fire-and-forget; the process is detached).
pub fn speak(text: &str, voice: &str) -> Result<()> {
    speak_impl(text, voice, false)
}

#[cfg(windows)]
fn speak_impl(text: &str, voice: &str, wait: bool) -> Result<()> {
    let trimmed: String = text.chars().take(600).collect();
    let ps = if voice.is_empty() {
        format!(
            "Add-Type -AssemblyName System.Speech; \
             $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
             $s.Speak(@'\n{}\n'@);",
            trimmed.replace('\'', "''")
        )
    } else {
        format!(
            "Add-Type -AssemblyName System.Speech; \
             $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
             $s.SelectVoice('{}'); \
             $s.Speak(@'\n{}\n'@);",
            voice.replace('\'', "''"),
            trimmed.replace('\'', "''")
        )
    };

    let mut cmd = Command::new("powershell");
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &ps]);
    let mut child = cmd.spawn().context("failed to start SAPI TTS")?;
    if wait {
        child.wait()?;
    } else {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
    Ok(())
}

#[cfg(not(windows))]
fn speak_impl(_text: &str, _voice: &str, _wait: bool) -> Result<()> {
    anyhow::bail!("TTS requires Windows (SAPI)")
}

/// Installed SAPI voices — backs the `tts_voice` setting's picker.
#[cfg(windows)]
#[allow(dead_code)] // the picker is not built yet; the setting already accepts a value
pub fn list_voices() -> Vec<String> {
    let out = Command::new("powershell")
        .creation_flags(CREATE_NO_WINDOW)
        .args(["-NoProfile", "-Command",
            "Add-Type -AssemblyName System.Speech; (New-Object System.Speech.Synthesis.SpeechSynthesizer).GetInstalledVoices() | ForEach-Object { $_.VoiceInfo.Name }"])
        .output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
        Err(_) => Vec::new(),
    }
}

#[cfg(not(windows))]
#[allow(dead_code)]
pub fn list_voices() -> Vec<String> {
    Vec::new()
}
