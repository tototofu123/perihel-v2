//! DictationController — the state machine wiring hotkey → audio → STT → format → inject.
//! Runs on a plain thread; talks to the HUD through shared state.

use crate::audio::{write_wav, Recorder};
use crate::hotkey::{HotkeyEvent, HotkeyMonitor};
use crate::injector::{Phase, TextInjector};
use crate::stt;
use anyhow::Result;
use perihel_core::{format_text, Settings};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct HudState {
    pub phase: Phase,
    pub started_at: Option<SystemTime>,
    pub rms: Arc<Mutex<f32>>,
    pub last_error: Option<String>,
}

impl HudState {
    fn new() -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self {
            phase: Phase::Idle,
            started_at: None,
            rms: Arc::new(Mutex::new(0.0)),
            last_error: None,
        }))
    }
}

/// Global counter of finished dictations, so the main Perihel app (or tests)
/// can observe activity without IPC.
pub static DICTATIONS_DONE: AtomicU64 = AtomicU64::new(0);

pub fn run(settings: Settings) -> Result<()> {
    let hud = HudState::new();
    let hotkey = HotkeyMonitor::new(&settings.dictation_hotkey)?;
    let _tray = crate::ui::tray::Tray::new()?;

    // controller thread — owns recorder and injector
    {
        let hud = hud.clone();
        let engine = settings.stt_engine.clone();
        std::thread::spawn(move || {
            if let Err(e) = controller_loop(hud, engine, hotkey) {
                log::error!("controller loop exited: {e:#}");
            }
        });
    }

    crate::ui::hud::run(hud) // main thread — egui
}

fn controller_loop(hud: Arc<Mutex<HudState>>, engine: String, hotkey: HotkeyMonitor) -> Result<()> {
    let mut recorder: Option<Recorder> = None;
    let mut injector = TextInjector::new()?;
    let data_dir = perihel_core::app_data_dir().join("voice");
    std::fs::create_dir_all(&data_dir).ok();

    while let Some(event) = hotkey.next_event() {
        match event {
            HotkeyEvent::Pressed if recorder.is_none() => {
                match Recorder::new().and_then(|r| {
                    r.start()?;
                    Ok(r)
                }) {
                    Ok(r) => {
                        recorder = Some(r);
                        set_phase(&hud, Phase::Listening);
                    }
                    Err(e) => {
                        log::warn!("failed to start recording: {e:#}");
                        set_error(&hud, format!("mic unavailable: {e}"));
                    }
                }
            }
            HotkeyEvent::Released if recorder.is_some() => {
                let mut rec = recorder.take().unwrap();
                set_phase(&hud, Phase::Transcribing);
                let samples = rec.stop();

                if samples.len() < 1600 {
                    // <0.1 s — noise; skip silently
                    set_phase(&hud, Phase::Idle);
                    continue;
                }

                let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
                let wav = data_dir.join(format!("dictation_{stamp}.wav"));
                if let Err(e) = write_wav(&wav, &samples) {
                    log::warn!("wav write failed: {e}");
                    set_error(&hud, "failed to buffer audio".into());
                    set_phase(&hud, Phase::Idle);
                    continue;
                }

                match stt::transcribe_wav(&wav, &engine) {
                    Ok(raw) => {
                        let text = format_text(&raw);
                        log::info!("transcribed: {text}");
                        if let Err(e) = injector.inject(&text) {
                            log::warn!("injection failed: {e:#}");
                            set_error(&hud, format!("injection failed: {e}"));
                        }
                        DICTATIONS_DONE.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(e) => {
                        log::warn!("transcription failed: {e:#}");
                        set_error(&hud, format!("transcription failed: {e}"));
                    }
                }
                // keep the raw wav around for debugging; prune anything older than a day
                prune_old_wavs(&data_dir);
                set_phase(&hud, Phase::Idle);
            }
            _ => {}
        }
    }
    Ok(())
}

fn set_phase(hud: &Arc<Mutex<HudState>>, phase: Phase) {
    if let Ok(mut h) = hud.lock() {
        h.phase = phase;
        h.started_at = matches!(phase, Phase::Listening).then(SystemTime::now);
        if phase == Phase::Idle {
            h.last_error = None;
        }
    }
}

fn set_error(hud: &Arc<Mutex<HudState>>, msg: String) {
    if let Ok(mut h) = hud.lock() {
        h.last_error = Some(msg);
    }
}

fn prune_old_wavs(dir: &std::path::Path) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        let cutoff = SystemTime::now() - Duration::from_secs(24 * 3600);
        for e in entries.flatten() {
            if let Ok(meta) = e.metadata() {
                if let Ok(modified) = meta.modified() {
                    if modified < cutoff {
                        std::fs::remove_file(e.path()).ok();
                    }
                }
            }
        }
    }
}
