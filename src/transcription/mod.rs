//! TranscriptionEngine trait + AudioChunk (mirrors murmur TranscriptionEngine.swift)

pub mod sherpa;

use anyhow::Result;

#[derive(Debug, Clone)]
pub struct AudioChunk {
    pub samples: Vec<i16>, // 16kHz mono i16
    pub seq: u64,
    pub rms: f32,
}

#[async_trait::async_trait]
pub trait TranscriptionEngine: Send {
    async fn transcribe(&mut self, chunks: Vec<AudioChunk>) -> Result<String>;
}
