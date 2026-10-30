use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// What the agent keeps between runs once it is paired.
#[derive(Serialize, Deserialize, Clone)]
pub struct Config {
    /// The API, e.g. https://restora.live/api/v1
    pub api: String,
    /// The agent's own key; the backend keeps only its hash.
    pub token: String,
}

/// `%APPDATA%\Restora` on Windows (`~/.config/restora` elsewhere, for development).
pub fn folder() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    let folder = base.unwrap_or_else(|| PathBuf::from(".")).join("restora");
    let _ = std::fs::create_dir_all(&folder);
    folder
}

pub fn load() -> Option<Config> {
    serde_json::from_slice(&std::fs::read(folder().join("agent.json")).ok()?).ok()
}

pub fn save(config: &Config) -> std::io::Result<()> {
    std::fs::write(folder().join("agent.json"), serde_json::to_vec_pretty(config)?)
}

pub fn clear() {
    let _ = std::fs::remove_file(folder().join("agent.json"));
}
