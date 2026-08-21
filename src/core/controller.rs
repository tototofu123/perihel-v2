//! DictationController — state machine, wires everything (mirrors murmur DictationController)

use crate::{
    core::{audio::AudioCapture, hotkey::HotkeyMonitor, injector::TextInjector},
    formatting::{dictionary::DictionaryCorrector, RuleBasedFormatter, TextFormatter},
    support::settings::Settings,
    transcription::{AudioChunk, TranscriptionEngine},
};
use anyhow::Result;
use log::{info, warn};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Idle,
    Starting,
    Listening,
    Finishing,
}

pub struct DictationController {
    state: State,
    settings: Settings,
    audio: AudioCapture,
    injector: TextInjector,
    formatter: RuleBasedFormatter,
    corrector: DictionaryCorrector,
}

impl DictationController {
    pub async fn new(settings: Settings) -> Result<(Self, crate::ui::hud::HudHandle)> {
        let audio = AudioCapture::new()?;
        let injector = TextInjector::new();
        let formatter = RuleBasedFormatter::new();
        let corrector = DictionaryCorrector::load_or_default();
        let hud_handle = crate::ui::hud::HudHandle::new();

        Ok((
            Self {
                state: State::Idle,
                settings,
                audio,
                injector,
                formatter,
                corrector,
            },
            hud_handle,
        ))
    }

    /// Main loop: consumes hotkey events, drives audio → transcription → injection
    /// AudioChunks flow through ordered mpsc (single consumer) — no Task-per-buffer.
    pub async fn run(mut self, mut hotkey: HotkeyMonitor) -> Result<()> {
        let (tx, mut rx) = mpsc::channel::<AudioChunk>(128);

        while let Some(evt) = hotkey.next_event().await {
            match evt {
                crate::core::hotkey::HotkeyEvent::Pressed if self.state == State::Idle => {
                    info!("hotkey down → starting");
                    self.state = State::Starting;
                    // Capture foreground HWND BEFORE showing HUD (else HUD steals it)
                    let target_hwnd = self.injector.capture_foreground();
                    self.audio.start(tx.clone()).await?;
                    self.state = State::Listening;
                    // HUD show handled via HudHandle
                }
                crate::core::hotkey::HotkeyEvent::Released if self.state == State::Listening => {
                    info!("hotkey up → finishing");
                    self.state = State::Finishing;
                    let chunks = self.audio.stop().await;
                    if chunks.is_empty() {
                        warn!("silence — injecting nothing");
                        self.state = State::Idle;
                        continue;
                    }
                    // Transcribe (sherpa int8, CPU)
                    let mut engine = crate::transcription::sherpa::SherpaParakeetEngine::new(&self.settings).await?;
                    let raw = engine.transcribe(chunks).await?;
                    if raw.trim().is_empty() {
                        self.state = State::Idle;
                        continue;
                    }
                    // Format + dictionary correction (longest-first, optional [\s-])
                    let mut text = self.formatter.format(&raw);
                    text = self.corrector.correct(&text);
                    info!("injecting {} chars", text.len());
                    self.injector.inject(&text)?;
                    self.state = State::Idle;
                }
                _ => {}
            }
        }
        Ok(())
    }
}
