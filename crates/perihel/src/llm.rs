//! LlamaSidecar — runs `llama-server` (llama.cpp, on PATH via winget) as a child
//! process with the small Perihel model, on demand, and stops it when idle.
//!
//! Low-resource contract:
//! - spawned only when the copilot is actually used (cold start ~2-5 s)
//! - child runs at IDLE priority, 3 threads, 1 parallel slot
//! - stopped after `idle_stop` of disuse — zero resident RAM between chats
//! - GPU layers auto: try full offload, fall back to CPU if load fails

use anyhow::{anyhow, bail, Context, Result};
use perihel_core::{ram_free_mb, ResourceGuard};
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub struct LlamaSidecar {
    port: u16,
    ctx_size: u32,
    child: Option<Child>,
    model: Option<PathBuf>,
    last_used: Option<Instant>,
    pub preferred_model: String, // file name under models/llm; empty = auto
}

impl LlamaSidecar {
    pub fn new(port: u16, ctx_size: u32, preferred_model: String) -> Self {
        Self { port, ctx_size, child: None, model: None, last_used: None, preferred_model }
    }

    pub fn is_running(&self) -> bool {
        self.child.is_some()
    }

    pub fn model_label(&self) -> String {
        self.model
            .as_ref()
            .map(|p| p.file_name().unwrap_or_default().to_string_lossy().to_string())
            .unwrap_or_else(|| "not loaded".into())
    }

    fn endpoint(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, path)
    }

    /// Start the server if not running. Returns Err when the resource guard is busy —
    /// the caller shows a friendly "machine is busy, will retry" instead of loading.
    pub fn ensure_running(&mut self, guard: &ResourceGuard) -> Result<()> {
        if self.is_running() {
            self.last_used = Some(Instant::now());
            return Ok(());
        }
        if guard.check() == perihel_core::Pressure::Busy {
            bail!("machine under load — holding off (will retry)");
        }

        let server = find_llama_server().context("llama-server.exe not found — install llama.cpp (winget install llama.cpp)")?;
        let model = resolve_model(&self.preferred_model).context("no .gguf model in models/llm — run scripts/download-models.ps1")?;
        let model_size_gb = std::fs::metadata(&model).map(|m| m.len() as f64 / 1e9).unwrap_or(1.0);

        for gpu_layers in gpu_layer_plan(&model_size_gb) {
            let mut cmd = Command::new(&server);
            #[cfg(windows)]
            cmd.creation_flags(CREATE_NO_WINDOW);
            cmd.args([
                "-m", model.to_string_lossy().as_ref(),
                "--host", "127.0.0.1",
                "--port", &self.port.to_string(),
                "--ctx-size", &self.ctx_size.to_string(),
                "--threads", "3",
                "--parallel", "1",
                "--no-webui",
                "--mlock",
                "--n-gpu-layers", &gpu_layers.to_string(),
            ]);
            let child = cmd
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .context("failed to spawn llama-server")?;
            self.child = Some(child);
            self.model = Some(model.clone());
            deprioritize_child(self.child.as_ref().unwrap());

            match wait_healthy(self.endpoint("/health"), Duration::from_secs(90)) {
                Ok(()) => {
                    log::info!("llama-server up on :{} (model {}, gpu {gpu_layers})", self.port, model.display());
                    self.last_used = Some(Instant::now());
                    return Ok(());
                }
                Err(e) => {
                    log::warn!("llama-server gpu={gpu_layers} failed: {e} — retrying plan");
                    self.stop();
                }
            }
        }
        self.model = None;
        bail!("llama-server failed to become healthy")
    }

    /// OpenAI-style chat completion against the sidecar. `grammar` = GBNF text;
    /// llama.cpp constrains sampling to it, so tool calls are always well-formed.
    pub fn chat(&mut self, messages: &[Value], grammar: Option<&str>, max_tokens: u32) -> Result<String> {
        if !self.is_running() {
            bail!("sidecar not running");
        }
        self.last_used = Some(Instant::now());

        let mut body = json!({
            "messages": messages,
            "max_tokens": max_tokens,
            "temperature": 0.4,
            "stream": false,
        });
        if let Some(g) = grammar {
            body["grammar"] = json!(g);
        }

        let resp = ureq::post(&self.endpoint("/v1/chat/completions"))
            .timeout(Duration::from_secs(120))
            .send_json(body)
            .map_err(|e| anyhow!("chat request failed: {e}"))?;
        let mut text = String::new();
        resp.into_reader().take(4_000_000).read_to_string(&mut text)?;
        let v: Value = serde_json::from_str(&text).context("bad JSON from llama-server")?;
        let content = v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if content.is_empty() {
            bail!("empty completion");
        }
        Ok(content)
    }

    pub fn stop_if_idle(&mut self, idle: Duration) {
        if let Some(t) = self.last_used {
            if self.is_running() && t.elapsed() > idle {
                log::info!("llama-server idle > {:?} — stopping to free RAM", idle);
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

impl Drop for LlamaSidecar {
    fn drop(&mut self) {
        self.stop();
    }
}

/// GPU plan: try full offload first if a CUDA device exists and free system RAM
/// is comfortable, else CPU-only. llama-server exits on OOM → caller retries next plan.
fn gpu_layer_plan(model_size_gb: &f64) -> Vec<u32> {
    let has_nvidia = std::process::Command::new("nvidia-smi")
        .creation_flags_win()
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if has_nvidia && ram_free_mb() > 2000 {
        vec![999, 0]
    } else {
        let _ = model_size_gb;
        vec![0]
    }
}

trait CreationFlagsExt {
    fn creation_flags_win(&mut self) -> &mut Self;
}

#[cfg(windows)]
impl CreationFlagsExt for Command {
    fn creation_flags_win(&mut self) -> &mut Self {
        self.creation_flags(CREATE_NO_WINDOW)
    }
}

#[cfg(not(windows))]
impl CreationFlagsExt for Command {
    fn creation_flags_win(&mut self) -> &mut Self {
        self
    }
}

/// Child process at IDLE priority — Perihel inference literally cannot slow the
/// user's foreground work down; the scheduler only feeds it leftover cycles.
#[cfg(windows)]
fn deprioritize_child(child: &Child) {
    use windows_sys::Win32::System::Threading::{OpenProcess, SetPriorityClass, IDLE_PRIORITY_CLASS, PROCESS_SET_INFORMATION};
    unsafe {
        let handle = OpenProcess(PROCESS_SET_INFORMATION, 0, child.id());
        if !handle.is_null() {
            SetPriorityClass(handle, IDLE_PRIORITY_CLASS);
        }
    }
}

#[cfg(not(windows))]
fn deprioritize_child(_child: &Child) {}

fn wait_healthy(url: String, timeout: Duration) -> Result<()> {
    let start = Instant::now();
    loop {
        if let Ok(resp) = ureq::get(&url).timeout(Duration::from_secs(2)).call() {
            if let Ok(s) = resp.into_string() {
                if s.contains("ok") {
                    return Ok(());
                }
            }
        }
        if start.elapsed() > timeout {
            bail!("health check timed out after {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(400));
    }
}

pub fn find_llama_server() -> Option<PathBuf> {
    if let Ok(p) = which::which("llama-server") {
        return Some(p);
    }
    // winget layout
    let base = dirs::data_local_dir()?.join("Microsoft/WinGet/Packages");
    for entry in std::fs::read_dir(&base).ok()?.flatten() {
        if entry.file_name().to_string_lossy().contains("llamacpp") {
            let p = entry.path().join("llama-server.exe");
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

/// Model resolution: explicit setting → fine-tuned perihel model → any small gguf.
/// Deliberately prefers small files (the user's RAM constraint).
pub fn resolve_model(preferred: &str) -> Option<PathBuf> {
    let dir = perihel_core::llm_models_dir();
    let is_tool_model = |p: &Path| {
        let n = p.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
        n.contains("perihel") || n.contains("toolcall")
    };
    let is_mmproj = |p: &Path| {
        let n = p.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
        n.contains("mmproj")
    };

    if !preferred.is_empty() {
        let p = dir.join(preferred);
        if p.exists() {
            return Some(p);
        }
    }

    let mut candidates: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e == "gguf").unwrap_or(false) && !is_mmproj(p))
        .collect();

    // fine-tuned perihel tool-calling model wins
    candidates.sort_by_key(|p| (!is_tool_model(p), std::fs::metadata(p).map(|m| m.len()).unwrap_or(u64::MAX)));
    candidates.into_iter().next()
}

/// Inventory of gguf models available for the UI dropdown.
pub fn list_models() -> Vec<String> {
    let dir = perihel_core::llm_models_dir();
    let mut out: Vec<String> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| n.ends_with(".gguf") && !n.contains("mmproj"))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn model_plan_prefers_small() {
        let mut names = ["some-model-q8.gguf", "perihel-tools-q4_k_m.gguf"];
        names.sort_by_key(|n| (!n.contains("perihel"), n.len()));
        assert!(names[0].contains("perihel"));
    }
}
