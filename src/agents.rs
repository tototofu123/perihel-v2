use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub path: Option<String>,
    pub config_dir: Option<String>,
    pub detected: bool,
}

pub static KNOWN_AGENTS: &[(&str, &str, &[&str], &[&str])] = &[
    ("codex", "OpenAI Codex", &["codex"], &[".codex"]),
    ("claude-code", "Claude Code", &["claude"], &[".claude"]),
    ("opencode", "OpenCode", &["opencode"], &[".opencode"]),
    ("gemini", "Gemini CLI", &["gemini"], &[".gemini"]),
    ("cursor-agent", "Cursor Agent", &["cursor-agent"], &[".cursor"]),
    ("copilot", "GitHub Copilot CLI", &["copilot"], &[".config/github-copilot"]),
    ("aider", "Aider", &["aider"], &[".aider"]),
    ("antigravity", "Antigravity", &["antigravity"], &[".antigravity"]),
    ("vox", "Vox TTS", &["vox"], &[".vox"]),
];

pub fn detect_agents() -> Vec<AgentInfo> {
    let mut agents = Vec::new();

    for (id, name, bins, config_dirs) in KNOWN_AGENTS {
        let (path, version) = detect_by_which(bins);
        let config_dir = detect_config_dir(config_dirs);

        let detected = path.is_some() || config_dir.is_some();
        agents.push(AgentInfo {
            id: id.to_string(),
            name: name.to_string(),
            version,
            path,
            config_dir,
            detected,
        });
    }

    agents
}

fn detect_by_which(bins: &[&str]) -> (Option<String>, Option<String>) {
    for bin in bins {
        if let Ok(path) = which::which(bin) {
            let path_str = path.to_string_lossy().to_string();
            let version = detect_version(bin);
            return (Some(path_str), version);
        }
    }
    (None, None)
}

fn detect_version(bin: &str) -> Option<String> {
    let mut child = std::process::Command::new(bin)
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    // Wait with 5s timeout to avoid hanging on unresponsive binaries
    let start = std::time::Instant::now();
    loop {
        if start.elapsed() > std::time::Duration::from_secs(5) {
            let _ = child.kill();
            return None;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    use std::io::Read;
                    let mut stdout = String::new();
                    child.stdout.take()?.read_to_string(&mut stdout).ok()?;
                    let line = stdout.lines().next().unwrap_or("").to_string();
                    if !line.is_empty() {
                        return Some(line);
                    }
                }
                return None;
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(_) => return None,
        }
    }
}

fn detect_config_dir(config_dirs: &[&str]) -> Option<String> {
    let home = dirs::home_dir()?;
    for dir in config_dirs {
        let path = home.join(dir);
        if path.exists() {
            return Some(path.to_string_lossy().to_string());
        }
    }
    None
}

pub fn get_env_agent_map() -> HashMap<&'static str, &'static str> {
    let mut map = HashMap::new();
    map.insert("CODEX_THREAD_ID", "codex");
    map.insert("CLAUDECODE", "claude-code");
    map.insert("OPENCODE", "opencode");
    map.insert("GEMINI_CLI", "gemini");
    map.insert("ANTIGRAVITY_AGENT", "antigravity");
    map
}

pub fn detect_by_env() -> Option<String> {
    let map = get_env_agent_map();
    for (env_var, agent_id) in &map {
        if std::env::var(env_var).is_ok() {
            return Some(agent_id.to_string());
        }
    }
    None
}
