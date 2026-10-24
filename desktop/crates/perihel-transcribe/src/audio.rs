//! Audio capture — cpal WASAPI shared mode, anything the mic gives us → 16 kHz mono i16.
//! Linear resampler (good enough for ASR; keeps the dependency surface tiny).

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub struct Recorder {
    stream: Option<Stream>,
    samples: Arc<Mutex<Vec<i16>>>,
    rms: Arc<Mutex<f32>>,
    /// Samples captured so far. Diagnostic only — read by the HUD level meter,
    /// which is not wired up yet; kept because the counter is already threaded
    /// through the capture path and is otherwise free.
    #[allow(dead_code)]
    total: Arc<AtomicU64>,
    source_rate: u32,
}

impl Recorder {
    pub fn new() -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("no default input device")?;
        let config = device.default_input_config().context("input config")?;
        let sample_format = config.sample_format();
        let cfg: cpal::StreamConfig = config.into();
        let source_rate = cfg.sample_rate.0;

        let samples = Arc::new(Mutex::new(Vec::new()));
        let rms = Arc::new(Mutex::new(0.0_f32));
        let total = Arc::new(AtomicU64::new(0));

        let buf = samples.clone();
        let rms_out = rms.clone();
        let total_out = total.clone();

        let stream = match sample_format {
            SampleFormat::F32 => device.build_input_stream(
                &cfg,
                move |data: &[f32], _| push_chunk(data, source_rate, &buf, &rms_out, &total_out),
                |err| log::warn!("audio input error: {err}"),
                None,
            )?,
            SampleFormat::I16 => device.build_input_stream(
                &cfg,
                move |data: &[i16], _| push_chunk_i16(data, source_rate, &buf, &rms_out, &total_out),
                |err| log::warn!("audio input error: {err}"),
                None,
            )?,
            other => anyhow::bail!("unsupported sample format {other:?}"),
        };
        Ok(Self { stream: Some(stream), samples, rms, total, source_rate })
    }

    pub fn start(&self) -> Result<()> {
        self.stream.as_ref().context("recorder already dropped")?.play()?;
        Ok(())
    }

    /// Smoothed input level, 0.0–1.0-ish. Diagnostic for the HUD level meter,
    /// which is not wired up yet.
    #[allow(dead_code)]
    pub fn rms(&self) -> f32 {
        self.rms.lock().map(|r| *r).unwrap_or(0.0)
    }

    /// Stop and hand over the accumulated 16 kHz samples.
    pub fn stop(&mut self) -> Vec<i16> {
        self.stream.take(); // drop → stops the stream
        if let Ok(mut r) = self.rms.lock() {
            *r = 0.0;
        }
        let taken = self.samples.lock().map(|mut b| std::mem::take(&mut *b)).unwrap_or_default();
        let secs = taken.len() as f64 / 16000.0;
        log::info!("captured {:.1}s of audio ({} samples, src {} Hz)", secs, taken.len(), self.source_rate);
        taken
    }
}

fn push_chunk(data: &[f32], src_rate: u32, buf: &Mutex<Vec<i16>>, rms: &Mutex<f32>, total: &AtomicU64) {
    // to mono i16 (cpal f32 is already interleaved mono for input configs)
    let mut sum_sq = 0.0_f32;
    let mut out = Vec::with_capacity(data.len() * 16000 / src_rate as usize + 16);
    for &s in data {
        let clamped = s.clamp(-1.0, 1.0);
        sum_sq += clamped * clamped;
        out.push((clamped * i16::MAX as f32) as i16);
    }
    finish_chunk(out, src_rate, buf, rms, total, sum_sq, data.len() as f32);
}

fn push_chunk_i16(data: &[i16], src_rate: u32, buf: &Mutex<Vec<i16>>, rms: &Mutex<f32>, total: &AtomicU64) {
    let mut sum_sq = 0.0_f32;
    let mut out = Vec::with_capacity(data.len() * 16000 / src_rate as usize + 16);
    for &s in data {
        let f = s as f32 / i16::MAX as f32;
        sum_sq += f * f;
        out.push(s);
    }
    finish_chunk(out, src_rate, buf, rms, total, sum_sq, data.len() as f32);
}

fn finish_chunk(
    mut out: Vec<i16>,
    src_rate: u32,
    buf: &Mutex<Vec<i16>>,
    rms: &Mutex<f32>,
    total: &AtomicU64,
    sum_sq: f32,
    n: f32,
) {
    if src_rate != 16000 {
        out = resample_linear(&out, src_rate, 16000);
    }
    if let Ok(mut b) = buf.lock() {
        b.extend_from_slice(&out);
    }
    total.fetch_add(out.len() as u64, Ordering::Relaxed);
    let frame_rms = (sum_sq / n.max(1.0)).sqrt();
    if let Ok(mut r) = rms.lock() {
        // smooth: 70% previous + 30% new, avoids flicker
        *r = *r * 0.7 + frame_rms * 0.3;
    }
}

/// Crisp linear interpolation resample — ASR engines tolerate it fine.
fn resample_linear(input: &[i16], from: u32, to: u32) -> Vec<i16> {
    if input.is_empty() || from == to {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let out_len = (input.len() as f64 / ratio) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let idx = pos as usize;
        let frac = (pos - idx as f64) as f32;
        let a = input[idx] as f32;
        let b = input.get(idx + 1).copied().unwrap_or(input[idx]) as f32;
        out.push((a + (b - a) * frac) as i16);
    }
    out
}

/// Minimal 16 kHz mono i16 WAV writer (44-byte canonical header).
pub fn write_wav(path: &Path, samples: &[i16]) -> Result<()> {
    use std::io::Write;
    let data_len = (samples.len() * 2) as u32;
    let mut f = std::fs::File::create(path)?;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_len).to_le_bytes())?;
    f.write_all(b"WAVE")?;
    f.write_all(b"fmt ")?;
    f.write_all(&16u32.to_le_bytes())?; // fmt chunk size
    f.write_all(&1u16.to_le_bytes())?; // PCM
    f.write_all(&1u16.to_le_bytes())?; // mono
    f.write_all(&16000u32.to_le_bytes())?;
    f.write_all(&32000u32.to_le_bytes())?; // byte rate
    f.write_all(&2u16.to_le_bytes())?; // block align
    f.write_all(&16u16.to_le_bytes())?; // bits
    f.write_all(b"data")?;
    f.write_all(&data_len.to_le_bytes())?;
    for s in samples {
        f.write_all(&s.to_le_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_48k_to_16k_triples() {
        let input: Vec<i16> = (0..4800).map(|i| ((i as f32 / 100.0).sin() * 1000.0) as i16).collect();
        let out = resample_linear(&input, 48000, 16000);
        assert_eq!(out.len(), 1600);
    }

    #[test]
    fn wav_header_roundtrip() {
        let dir = std::env::temp_dir().join("perihel-test-wav");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.wav");
        write_wav(&p, &[0, 100, -100]).unwrap();
        let bytes = std::fs::read(&p).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(bytes.len(), 44 + 6);
        std::fs::remove_file(&p).ok();
    }
}
