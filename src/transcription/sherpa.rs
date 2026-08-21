//! SherpaParakeetEngine — sherpa-onnx + Parakeet TDT 0.6B int8 (CPU, 66MB resident)
//! Bundles ONNX Runtime, no GPU, ~40x realtime

use super::{AudioChunk, TranscriptionEngine};
use crate::support::settings::Settings;
use anyhow::Result;
use log::info;

pub struct SherpaParakeetEngine {
    // sherpa_rs::OfflineRecognizer held here
}

impl SherpaParakeetEngine {
    pub async fn new(settings: &Settings) -> Result<Self> {
        let model_dir = crate::support::paths::model_dir().join("parakeet-tdt-0.6b-int8");
        if !model_dir.exists() {
            info!("downloading Parakeet int8 to {:?}", model_dir);
            Self::download_model(&model_dir).await?;
        }
        // Real init:
        // let config = sherpa_rs::OfflineRecognizerConfig { ... encoder, decoder, joiner, tokens ... };
        // let recognizer = sherpa_rs::OfflineRecognizer::from_config(config)?;
        info!("parakeet model ready at {:?}", model_dir);
        Ok(Self {})
    }

    async fn download_model(dir: &std::path::Path) -> Result<()> {
        // hf-hub download from istupakov/parakeet-tdt-0.6b-v2-int8-onnx
        // Files: encoder.int8.onnx, decoder.onnx, joiner.onnx, tokens.txt
        // Skeleton — real impl uses hf_hub::api::tokio::ApiBuilder
        tokio::fs::create_dir_all(dir).await?;
        info!("(skeleton) would download hf model to {:?}", dir);
        Ok(())
    }
}

#[async_trait::async_trait]
impl TranscriptionEngine for SherpaParakeetEngine {
    async fn transcribe(&mut self, chunks: Vec<AudioChunk>) -> Result<String> {
        // Concatenate ordered chunks (seq already ordered by single consumer)
        let mut pcm: Vec<f32> = Vec::new();
        for c in chunks {
            for s in c.samples { pcm.push(s as f32 / i16::MAX as f32); }
        }
        if pcm.is_empty() { return Ok(String::new()); }
        // let stream = recognizer.create_stream();
        // stream.accept_waveform(16000, &pcm);
        // recognizer.decode(&mut stream);
        // Ok(stream.get_result().text)
        Ok(String::from("(skeleton) transcription would happen here — need sherpa-rs linked)"))
    }
}
