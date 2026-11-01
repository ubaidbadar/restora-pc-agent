use crate::{config::{self, Config}, printers, slips};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashSet, VecDeque},
    io::Write,
    sync::Mutex,
    time::Duration,
};

/// The API when none is set (`RESTORA_API` replaces it).
const DEFAULT_API: &str = "https://restora.live/api/v1";
/// The APIs the web app may pair this agent with: Restora and its dev site. `RESTORA_API` is one too.
const KNOWN_APIS: [&str; 2] = [DEFAULT_API, "https://dev.restora.live/api/v1"];

/// The API to pair with: the one the web app that sent the code uses, when it is a Restora one, else the default.
pub fn api_for(asked: Option<&str>) -> Result<String, String> {
    let default = std::env::var("RESTORA_API").unwrap_or_else(|_| DEFAULT_API.to_string());
    match asked.map(|a| a.trim().trim_end_matches('/')) {
        None | Some("") => Ok(default),
        Some(api) if api == default || KNOWN_APIS.contains(&api) => Ok(api.to_string()),
        Some(_) => Err("That Restora address is not allowed".to_string()),
    }
}

/// What the local server and the backend loop share: who this agent is, and the slips already printed.
pub struct Agent {
    pub config: Mutex<Option<Config>>,
    /// `orderKey|kind|printer` of printed slips, kept in printed.log, so the same slip is never printed twice (also after a restart).
    printed: Mutex<HashSet<String>>,
    /// The printers as staff set them in Restora, kept (printers.json) so the web app on this PC can print without internet.
    printers: Mutex<serde_json::Value>,
    pub http: reqwest::Client,
    /// Whether the last call to Restora got through, for the setup page.
    pub online: std::sync::atomic::AtomicBool,
    recent: Mutex<VecDeque<Recent>>,
}

/// How many results the setup page lists.
const RECENT: usize = 8;

/// One slip's latest result, for the "Recent slips" list. Only kept while the program runs.
#[derive(Serialize, Clone)]
pub struct Recent {
    /// When it last changed, seconds since 1970.
    pub at: u64,
    pub printer: String,
    pub kind: String,
    /// "Order #12": what staff know the order by.
    pub order: String,
    /// "printed", or "failed" (it will be tried again until it is given up).
    pub state: &'static str,
    pub error: Option<String>,
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
        Self { config: Mutex::new(config::load()), printed: Mutex::new(printed), printers: Mutex::new(printers), http: reqwest::Client::builder().timeout(Duration::from_secs(15)).build().unwrap(), online: std::sync::atomic::AtomicBool::new(false), recent: Mutex::new(VecDeque::new()) }
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
        let sent = (0..slip.copies.clamp(1, 5)).try_for_each(|_| printers::print_raw(slip.printer, &bytes));
        self.note(slip, sent.as_ref().err().cloned());
        sent?;
        self.remember(Self::key(slip));
        Ok(true)
    }

    /// Keeps the result for the setup page; a slip tried again replaces its own line instead of adding one.
    fn note(&self, slip: &Slip, error: Option<String>) {
        let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let order = format!("Order #{}", slips::token(slip.order));
        let line = Recent { at, printer: slip.printer.to_string(), kind: slip.kind.to_string(), order, state: if error.is_some() { "failed" } else { "printed" }, error };
        let mut recent = self.recent.lock().unwrap();
        recent.retain(|r| !(r.order == line.order && r.kind == line.kind && r.printer == line.printer));
        recent.push_front(line);
        recent.truncate(RECENT);
    }

    pub fn recent(&self) -> Vec<Recent> {
        self.recent.lock().unwrap().iter().cloned().collect()
    }

    fn remember(&self, key: String) {
        if let Ok(mut log) = std::fs::OpenOptions::new().create(true).append(true).open(config::folder().join("printed.log")) {
            let _ = writeln!(log, "{key}");
        }
        self.printed.lock().unwrap().insert(key);
    }

    /// Pairs with the code from Settings, typed here or sent by the web app open on this PC (with the API it uses); the
    /// backend answers the key this agent uses from then on.
    pub async fn pair(&self, code: &str, api: Option<&str>) -> Result<(), String> {
        let api = api_for(api)?;
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

    #[test]
    fn pairs_only_with_restora_apis() {
        assert_eq!(api_for(None).unwrap(), DEFAULT_API);
        assert_eq!(api_for(Some("https://dev.restora.live/api/v1/")).unwrap(), "https://dev.restora.live/api/v1");
        assert!(api_for(Some("https://evil.example/api/v1")).is_err());
    }
}
