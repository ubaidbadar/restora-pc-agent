use crate::{config::{self, Config}, printers, slips};
use serde::Deserialize;
use std::{
    collections::HashSet,
    io::Write,
    sync::Mutex,
    time::Duration,
};

/// The API when none is set (`RESTORA_API` replaces it).
const DEFAULT_API: &str = "https://restora.live/api/v1";

/// What the local server and the backend loop share: who this agent is, and the slips already printed.
pub struct Agent {
    pub config: Mutex<Option<Config>>,
    /// `orderKey|kind|printer` of printed slips, kept in printed.log, so the same slip is never printed twice (also after a restart).
    printed: Mutex<HashSet<String>>,
    /// The printers as staff set them in Restora, kept (printers.json) so the web app on this PC can print without internet.
    printers: Mutex<serde_json::Value>,
    pub http: reqwest::Client,
}

/// A slip to print, from the web app on this PC or from the backend.
pub struct Slip<'a> {
    pub printer: &'a str,
    pub kind: &'a str,
    pub order_key: &'a str,
    pub width: usize,
    pub copies: u32,
    pub order: &'a slips::Order,
    pub header: &'a slips::Header,
}

#[derive(Deserialize)]
struct Paired {
    token: String,
    #[serde(default)]
    printers: serde_json::Value,
}

impl Agent {
    pub fn new() -> Self {
        let printed = std::fs::read_to_string(config::folder().join("printed.log")).unwrap_or_default().lines().map(String::from).collect();
        let printers = std::fs::read(config::folder().join("printers.json")).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_else(|| serde_json::json!([]));
        Self { config: Mutex::new(config::load()), printed: Mutex::new(printed), printers: Mutex::new(printers), http: reqwest::Client::builder().timeout(Duration::from_secs(15)).build().unwrap() }
    }

    pub fn paired(&self) -> bool {
        self.config.lock().unwrap().is_some()
    }

    fn key(slip: &Slip) -> String {
        format!("{}|{}|{}", slip.order_key, slip.kind, slip.printer)
    }

    /// Prints the slip unless it was printed before. Answers whether it printed now.
    pub fn print(&self, slip: &Slip) -> Result<bool, String> {
        if self.printed.lock().unwrap().contains(&Self::key(slip)) {
            return Ok(false);
        }
        let bytes = slips::render(slip.kind, slip.order, slip.header, slip.width).ok_or("kind must be kitchen or receipt")?;
        for _ in 0..slip.copies.clamp(1, 5) {
            printers::print_raw(slip.printer, &bytes)?;
        }
        self.remember(Self::key(slip));
        Ok(true)
    }

    fn remember(&self, key: String) {
        if let Ok(mut log) = std::fs::OpenOptions::new().create(true).append(true).open(config::folder().join("printed.log")) {
            let _ = writeln!(log, "{key}");
        }
        self.printed.lock().unwrap().insert(key);
    }

    /// Pairs with the code from Settings; the backend answers the key this agent uses from then on.
    pub async fn pair(&self, code: &str) -> Result<(), String> {
        let api = std::env::var("RESTORA_API").unwrap_or_else(|_| DEFAULT_API.to_string());
        let body = serde_json::json!({ "code": code, "version": env!("CARGO_PKG_VERSION"), "printers": printers::list().unwrap_or_default() });
        let response = self.http.post(format!("{api}/print-agent/pair")).json(&body).send().await.map_err(|e| format!("Cannot reach Restora: {e}"))?;
        if !response.status().is_success() {
            let message = response.json::<serde_json::Value>().await.ok().and_then(|v| v["message"].as_str().map(String::from));
            return Err(message.unwrap_or_else(|| "Pairing failed".to_string()));
        }
        let paired: Paired = response.json().await.map_err(|e| e.to_string())?;
        let config = Config { api, token: paired.token };
        config::save(&config).map_err(|e| format!("Cannot save the key: {e}"))?;
        *self.config.lock().unwrap() = Some(config);
        self.set_printers(paired.printers);
        Ok(())
    }

    pub fn printers(&self) -> serde_json::Value {
        self.printers.lock().unwrap().clone()
    }

    pub fn set_printers(&self, printers: serde_json::Value) {
        if !printers.is_array() {
            return;
        }
        let _ = std::fs::write(config::folder().join("printers.json"), printers.to_string());
        *self.printers.lock().unwrap() = printers;
    }

    /// The backend says this agent is no longer paired (it was removed in Settings): forget the key.
    pub fn unpair(&self) {
        config::clear();
        self.set_printers(serde_json::json!([]));
        *self.config.lock().unwrap() = None;
    }
}

/// Seconds to wait before trying a failed print again: 2, 5, 10, 30, then every minute. Past 15 minutes the slip is given up.
pub fn retry_delay(attempt: u32) -> Duration {
    Duration::from_secs(match attempt {
        0 => 2,
        1 => 5,
        2 => 10,
        3 => 30,
        _ => 60,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_get_slower_and_settle_at_a_minute() {
        let seconds: Vec<u64> = (0..6).map(|attempt| retry_delay(attempt).as_secs()).collect();
        assert_eq!(seconds, [2, 5, 10, 30, 60, 60]);
    }
}
