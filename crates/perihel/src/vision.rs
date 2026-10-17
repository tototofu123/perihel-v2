//! VisionSidecar — on-demand llama-server with an mmproj vision model.
//!
//! Only started when a capture actually asks to describe a screenshot, stopped
//! after 2 minutes idle. Model + mmproj live in models/vision (SmolVLM-256M class,
//! ~300 MB download, fits the small-model budget).

use anyhow::{anyhow, bail, Context, Result};
use perihel_core::{ResourceGuard};
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub struct VisionSidecar {
    port: u16,
    child: Option<Child>,
    model: Option<(PathBuf, PathBuf)>, // (gguf, mmproj)
    last_used: Option<Instant>,
}

impl VisionSidecar {
    pub fn new(port: u16) -> Self {
        Self { port, child: None, model: None, last_used: None }
    }

    pub fn is_running(&self) -> bool {
        self.child.is_some()
    }

    /// Model file currently loaded by the sidecar, for display in Settings.
    #[allow(dead_code)] // consumed by the Settings model picker, which is not built yet
    pub fn model_label(&self) -> String {
        self.model
            .as_ref()
            .map(|(m, _)| m.file_name().unwrap_or_default().to_string_lossy().to_string())
            .unwrap_or_else(|| "not loaded".into())
    }

    pub fn available(&self) -> bool {
        find_vision_pair().is_some()
    }

    fn ensure_running(&mut self, guard: &ResourceGuard) -> Result<()> {
        if self.is_running() {
            self.last_used = Some(Instant::now());
            return Ok(());
        }
        if guard.check() == perihel_core::Pressure::Busy {
            bail!("machine under load — vision deferred");
        }
        let server = super::llm::find_llama_server().context("llama-server.exe not found")?;
        let (model, mmproj) = find_vision_pair().context("no vision model in models/vision (need *.gguf + *mmproj*.gguf)")?;

        let mut cmd = Command::new(&server);
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd.args([
            "-m", model.to_string_lossy().as_ref(),
            "--mmproj", mmproj.to_string_lossy().as_ref(),
            "--host", "127.0.0.1",
            "--port", &self.port.to_string(),
            "--ctx-size", "2048",
            "--threads", "3",
            "--parallel", "1",
            "--no-webui",
            "--n-gpu-layers", "999",
        ]);
        let child = cmd
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context("failed to spawn vision llama-server")?;
        self.child = Some(child);
        self.model = Some((model, mmproj));

        // health wait (short — vision starts on demand, failing fast is fine)
        let url = format!("http://127.0.0.1:{}/health", self.port);
        let start = Instant::now();
        loop {
            if let Ok(resp) = ureq::get(&url).timeout(Duration::from_secs(2)).call() {
                if let Ok(s) = resp.into_string() {
                    if s.contains("ok") {
                        break;
                    }
                }
            }
            if start.elapsed() > Duration::from_secs(60) {
                self.stop();
                bail!("vision sidecar failed to start (gpu fallback not attempted)");
            }
            std::thread::sleep(Duration::from_millis(400));
        }
        self.last_used = Some(Instant::now());
        Ok(())
    }

    /// Describe the newest screenshot PNG under data_dir/screenshots.
    pub fn describe_latest(&mut self, data_dir: &Path, question: &str, guard: &ResourceGuard) -> Result<String> {
        self.ensure_running(guard)?;
        let shot = newest_screenshot(data_dir).context("no screenshots taken yet")?;
        let bytes = std::fs::read(&shot)?;
        let b64 = base64_encode(&bytes);

        let body = json!({
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": question},
                    {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{b64}")}}
                ]
            }],
            "max_tokens": 300,
            "temperature": 0.2,
            "stream": false,
        });
        let resp = ureq::post(&format!("http://127.0.0.1:{}/v1/chat/completions", self.port))
            .timeout(Duration::from_secs(90))
            .send_json(body)
            .map_err(|e| anyhow!("vision request failed: {e}"))?;
        let mut text = String::new();
        resp.into_reader().take(2_000_000).read_to_string(&mut text)?;
        let v: Value = serde_json::from_str(&text)?;
        Ok(v["choices"][0]["message"]["content"].as_str().unwrap_or_default().to_string())
    }

    pub fn stop_if_idle(&mut self, idle: Duration) {
        if let Some(t) = self.last_used {
            if self.is_running() && t.elapsed() > idle {
                log::info!("vision sidecar idle — stopping");
                self.stop();
            }
        }
    }

    pub fn stop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.model = None;
        self.last_used = None;
    }
}

impl Drop for VisionSidecar {
    fn drop(&mut self) {
        self.stop();
    }
}

fn find_vision_pair() -> Option<(PathBuf, PathBuf)> {
    let dir = perihel_core::vision_models_dir();
    let files: Vec<PathBuf> = std::fs::read_dir(&dir).ok()?.flatten().map(|e| e.path()).collect();
    let mmproj = files.iter().find(|p| {
        p.file_name().unwrap_or_default().to_string_lossy().to_lowercase().contains("mmproj")
    })?;
    let model = files.iter().find(|p| {
        p.extension().map(|e| e == "gguf").unwrap_or(false)
            && !p.file_name().unwrap_or_default().to_string_lossy().to_lowercase().contains("mmproj")
    })?;
    Some((model.clone(), mmproj.clone()))
}

fn newest_screenshot(data_dir: &Path) -> Option<PathBuf> {
    let dir = data_dir.join("screenshots");
    std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e == "png").unwrap_or(false))
        .max_by_key(|p| p.metadata().and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH))
}

fn base64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}
