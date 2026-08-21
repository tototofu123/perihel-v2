//! AudioCapture — WASAPI via cpal, 48kHz → 16kHz mono, ordered AudioChunk stream
//! Mirrors murmur AudioCapture: buffers copied, never borrowed, single consumer.

use crate::transcription::AudioChunk;
use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use log::info;
use tokio::sync::mpsc;

pub struct AudioCapture {
    // stream held while listening
    _stream: Option<cpal::Stream>,
    buffer: Vec<i16>,
    seq: u64,
}

impl AudioCapture {
    pub fn new() -> Result<Self> {
        Ok(Self { _stream: None, buffer: Vec::with_capacity(16000 * 2), seq: 0 })
    }

    pub async fn start(&mut self, tx: mpsc::Sender<AudioChunk>) -> Result<()> {
        let host = cpal::default_host();
        let device = host.default_input_device().expect("no input device");
        let config = device.default_input_config()?;
        info!("audio input: {:?} {:?}", device.name(), config);

        let tx2 = tx.clone();
        let mut seq = self.seq;

        // For brevity, skeleton: real impl uses ringbuf + rubato resampler 48k→16k, RMS per 10ms
        let stream = device.build_input_stream(
            &config.into(),
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                // Copy immediately — cpal buffer recycled after callback
                let mut samples = Vec::with_capacity(data.len());
                for &s in data {
                    samples.push((s * i16::MAX as f32) as i16);
                }
                let chunk = AudioChunk { samples, seq, rms: 0.0 };
                seq += 1;
                let _ = tx2.try_send(chunk);
            },
            |err| log::error!("audio error: {}", err),
            None,
        )?;
        stream.play()?;
        self._stream = Some(stream);
        Ok(())
    }

    pub async fn stop(&mut self) -> Vec<AudioChunk> {
        self._stream = None;
        // In real impl drain mpsc; here return empty for skeleton
        Vec::new()
    }

    pub fn rms(&self) -> f32 { 0.0 }
}
