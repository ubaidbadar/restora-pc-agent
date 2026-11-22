use crate::{
    config::{self, Config},
    printer_state::{self, PrinterHealth, WinPrinter},
    printers, slips,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::Write,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
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
    /// Prevent concurrent submissions to the same printer without blocking unrelated printers.
    printer_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// On unreadable history, fail closed instead of potentially repeating printed orders.
    journal_ok: AtomicBool,
    /// The printers as staff set them in Restora, kept (printers.json) so the web app on this PC can print without internet.
    printers: Mutex<serde_json::Value>,
    /// Last successful Windows printer discovery; status requests never call the blocking spooler API.
    windows_printers: Mutex<Option<Vec<WinPrinter>>>,
    health: Mutex<Option<Vec<PrinterHealth>>>,
    pnp_count: Mutex<Option<usize>>,
    pub refresh_pending: Arc<AtomicBool>,
    pub discovery_busy: Arc<AtomicBool>,
    /// Shared with the remote job dispatcher and explicit test printing.
    pub active_printers: Arc<Mutex<HashSet<String>>>,
    last_results: Mutex<HashMap<String, LastPrintResult>>,
    test_after: Mutex<HashMap<String, Instant>>,
    pub http: reqwest::Client,
    /// Whether the last call to Restora got through, for the local status endpoint.
    pub online: std::sync::atomic::AtomicBool,
    recent: Mutex<VecDeque<Recent>>,
}

/// How many results the local status endpoint lists.
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

/// Last spooler submission outcome, persisted independently from printed.log.
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastPrintResult {
    pub ok: bool,
    pub at: u64,
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
#[serde(rename_all = "camelCase")]
struct PairedAgent {
    branch_id: String,
}

#[derive(Deserialize)]
struct Paired {
    token: String,
    #[serde(default)]
    printers: serde_json::Value,
    #[serde(default)]
    agent: Option<PairedAgent>,
}

impl Agent {
    pub fn new() -> Self {
        let (printed, journal_ok) =
            match std::fs::read_to_string(config::folder().join("printed.log")) {
                Ok(contents) => (contents.lines().map(String::from).collect(), true),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => (HashSet::new(), true),
                Err(err) => {
                    tracing::error!("cannot read printed slip journal: {err}");
                    (HashSet::new(), false)
                }
            };
        let printers = std::fs::read(config::folder().join("printers.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_else(|| serde_json::json!([]));
        let last_results: HashMap<String, LastPrintResult> =
            std::fs::read(config::folder().join("printer-last-results.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default();
        Self {
            config: Mutex::new(config::load()),
            printed: Mutex::new(printed),
            printer_locks: Mutex::new(HashMap::new()),
            journal_ok: AtomicBool::new(journal_ok),
            printers: Mutex::new(printers),
            windows_printers: Mutex::new(None),
            health: Mutex::new(None),
            pnp_count: Mutex::new(None),
            refresh_pending: Arc::new(AtomicBool::new(true)),
            discovery_busy: Arc::new(AtomicBool::new(false)),
            active_printers: Arc::new(Mutex::new(HashSet::new())),
            last_results: Mutex::new(last_results),
            test_after: Mutex::new(HashMap::new()),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap(),
            online: std::sync::atomic::AtomicBool::new(false),
            recent: Mutex::new(VecDeque::new()),
        }
    }

    pub fn paired(&self) -> bool {
        self.config.lock().unwrap().is_some()
    }

    fn key(slip: &Slip) -> String {
        format!("{}|{}|{}", slip.order_key, slip.kind, slip.printer)
    }

    /// A durable copy marker is recorded after each accepted spooler submission.
    /// Existing full-slip entries remain valid when upgrading from earlier agents.
    /// A crash between spooler acceptance and this write remains ambiguous: no
    /// generic thermal printer can guarantee physical exactly-once delivery.
    pub fn print(&self, slip: &Slip) -> Result<bool, String> {
        if !self.journal_ok.load(Ordering::Relaxed) {
            return Err("Printed slip history cannot be read. Check agent.log and the disk before retrying.".into());
        }
        let lock = {
            let mut locks = self.printer_locks.lock().map_err(|e| e.to_string())?;
            Arc::clone(
                locks
                    .entry(slip.printer.to_string())
                    .or_insert_with(|| Arc::new(Mutex::new(()))),
            )
        };
        // Only this physical printer is serialized; other printers can accept jobs.
        let _printer = lock.lock().map_err(|e| e.to_string())?;
        let key = Self::key(slip);
        if self
            .printed
            .lock()
            .map_err(|e| e.to_string())?
            .contains(&key)
        {
            return Ok(false);
        }

        let bytes = slips::render(slip.kind, slip.order, slip.header, slip.width)
            .ok_or("kind must be kitchen or receipt")?;
        let mut submitted = false;
        for copy in 0..slip.copies.clamp(1, 5) {
            let marker = format!("{key}|copy:{copy}");
            if self
                .printed
                .lock()
                .map_err(|e| e.to_string())?
                .contains(&marker)
            {
                continue;
            }
            if let Err(err) = printers::print_raw(slip.printer, &bytes) {
                self.note(slip, Some(err.clone()));
                return Err(err);
            }
            submitted = true;
            // If persistence fails, remember the copy in this process to prevent
            // a retry from immediately printing it again.
            self.printed
                .lock()
                .map_err(|e| e.to_string())?
                .insert(marker.clone());
            if let Err(err) = Self::append_journal(&marker) {
                self.journal_ok.store(false, Ordering::Relaxed);
                let message = format!(
                    "Windows accepted the print job, but saving its history failed: {err}. Check the disk before restarting this agent."
                );
                self.note(slip, Some(message.clone()));
                return Err(message);
            }
        }

        // Keep the legacy completed-key format for old versions and dedupe.
        self.printed
            .lock()
            .map_err(|e| e.to_string())?
            .insert(key.clone());
        if let Err(err) = Self::append_journal(&key) {
            self.journal_ok.store(false, Ordering::Relaxed);
            let message =
                format!("Print copies submitted, but final history could not be saved: {err}");
            self.note(slip, Some(message.clone()));
            return Err(message);
        }
        self.note(slip, None);
        Ok(submitted)
    }

    fn append_journal(key: &str) -> Result<(), String> {
        let mut log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(config::folder().join("printed.log"))
            .map_err(|e| e.to_string())?;
        writeln!(log, "{key}").map_err(|e| e.to_string())?;
        log.sync_data().map_err(|e| e.to_string())
    }

    /// Keeps the result for the setup page; a slip tried again replaces its own line instead of adding one.
    fn note(&self, slip: &Slip, error: Option<String>) {
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let order = format!("Order #{}", slips::token(slip.order));
        let succeeded = error.is_none();
        let line = Recent {
            at,
            printer: slip.printer.to_string(),
            kind: slip.kind.to_string(),
            order,
            state: if error.is_some() { "failed" } else { "printed" },
            error,
        };
        let mut recent = self.recent.lock().unwrap();
        recent.retain(|r| {
            !(r.order == line.order && r.kind == line.kind && r.printer == line.printer)
        });
        recent.push_front(line);
        recent.truncate(RECENT);
        drop(recent);
        self.record_last_result(slip.printer, succeeded, at);
        if !succeeded {
            self.request_printer_refresh();
        }
    }

    fn record_last_result(&self, printer: &str, ok: bool, at: u64) {
        // Hold the map lock through the atomic rename: concurrent jobs on distinct
        // printers must not overwrite each other's state with an older snapshot.
        let mut results = self.last_results.lock().unwrap();
        results.insert(printer.to_string(), LastPrintResult { ok, at });
        if let Err(error) = Self::persist_last_results(&results) {
            tracing::warn!("could not persist printer's last result: {error}");
        }
    }

    fn persist_last_results(results: &HashMap<String, LastPrintResult>) -> Result<(), String> {
        use std::io::Write;
        let path = config::folder().join("printer-last-results.json");
        let temp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec(results).map_err(|e| e.to_string())?;
        let mut file = std::fs::File::create(&temp).map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        // std::fs::rename uses the platform replacement operation; never delete
        // the previous result before the replacement is prepared.
        std::fs::rename(temp, path).map_err(|e| e.to_string())
    }

    pub fn last_result(&self, printer: &str) -> Option<LastPrintResult> {
        self.last_results.lock().unwrap().get(printer).cloned()
    }

    pub fn request_printer_refresh(&self) {
        self.refresh_pending.store(true, Ordering::Release);
    }

    pub fn set_pnp_count(&self, count: usize) {
        *self.pnp_count.lock().unwrap() = Some(count);
    }

    pub fn pnp_count(&self) -> Option<usize> {
        *self.pnp_count.lock().unwrap()
    }

    pub fn printer_health(&self) -> Option<Vec<PrinterHealth>> {
        self.health.lock().unwrap().clone()
    }

    /// Explicit user-initiated RAW test; never touches the printed.log dedupe journal.
    pub fn test_print(&self, printer: &str) -> Result<(), String> {
        if self.active_printers.lock().unwrap().contains(printer) {
            return Err("A real job is printing on this printer".into());
        }
        {
            let mut after = self.test_after.lock().unwrap();
            if after
                .get(printer)
                .is_some_and(|previous| previous.elapsed() < Duration::from_secs(10))
            {
                return Err("Please wait 10 seconds between test prints".into());
            }
            after.insert(printer.to_string(), Instant::now());
        }
        {
            let mut active = self.active_printers.lock().unwrap();
            if !active.insert(printer.to_string()) {
                return Err("Printer busy".into());
            }
        }
        let result = (|| {
            if self
                .windows_printers
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|list| list.iter().any(|p| p.name == printer && p.jobs > 0))
            {
                return Err("Windows has pending printer jobs".to_string());
            }
            let lock = {
                let mut locks = self.printer_locks.lock().unwrap();
                Arc::clone(
                    locks
                        .entry(printer.to_string())
                        .or_insert_with(|| Arc::new(Mutex::new(()))),
                )
            };
            let _guard = lock.lock().map_err(|e| e.to_string())?;
            printers::print_raw(printer, b"\x1b@Restora test print\n\n\n\x1dV\x00")
        })();
        self.active_printers.lock().unwrap().remove(printer);
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let error = result.as_ref().err().cloned();
        {
            let mut recent = self.recent.lock().unwrap();
            recent.push_front(Recent {
                at,
                printer: printer.to_owned(),
                kind: "test".into(),
                order: "Manual test print".into(),
                state: if result.is_ok() { "printed" } else { "failed" },
                error,
            });
            recent.truncate(RECENT);
        }
        self.record_last_result(printer, result.is_ok(), at);
        if result.is_err() {
            self.request_printer_refresh();
        }
        result
    }

    pub fn recent(&self) -> Vec<Recent> {
        self.recent.lock().unwrap().iter().cloned().collect()
    }

    /// Pairs with the code from Settings, typed here or sent by the web app open on this PC (with the API it uses); the
    /// backend answers the key this agent uses from then on.
    pub async fn pair(&self, code: &str, api: Option<&str>) -> Result<(), String> {
        if self.paired() {
            return Err("This PC is already connected. Remove it from the original branch in Restora before reconnecting.".into());
        }
        let api = api_for(api)?;
        let mut printer_discovery_completed = false;
        let windows_printers = match tokio::time::timeout(
            Duration::from_secs(5),
            tokio::task::spawn_blocking(printers::list),
        )
        .await
        {
            Ok(Ok(Ok(names))) => {
                printer_discovery_completed = true;
                names
            }
            Ok(Ok(Err(error))) => {
                tracing::warn!("could not discover Windows printers while pairing: {error}");
                Vec::new()
            }
            Ok(Err(error)) => {
                tracing::warn!("Windows printer discovery task failed while pairing: {error}");
                Vec::new()
            }
            Err(_) => {
                tracing::warn!(
                    "Windows printer discovery timed out while pairing; it will retry in the background"
                );
                Vec::new()
            }
        };
        if printer_discovery_completed {
            self.set_windows_printers(windows_printers.clone());
        }
        let body = serde_json::json!({ "code": code, "version": env!("CARGO_PKG_VERSION"), "printers": printers::names(&windows_printers) });
        let response = self
            .http
            .post(format!("{api}/print-agent/pair"))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Cannot reach Restora: {e}"))?;
        if !response.status().is_success() {
            let message = response
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|v| v["message"].as_str().map(String::from));
            return Err(message.unwrap_or_else(|| "Pairing failed".to_string()));
        }
        let paired: Paired = response.json().await.map_err(|e| e.to_string())?;
        let config = Config {
            api,
            token: paired.token,
            branch_id: paired.agent.map(|agent| agent.branch_id),
        };
        config::save(&config).map_err(|e| format!("Cannot save the key: {e}"))?;
        config::clear_reconnect_required();
        // A new pairing must not reuse completed keys from another restaurant.
        // If old history cannot be cleared, disable printing rather than suppress
        // valid jobs or risk emitting a duplicate after a restart.
        match std::fs::remove_file(config::folder().join("printed.log")) {
            Ok(()) => self.printed.lock().unwrap().clear(),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                self.printed.lock().unwrap().clear()
            }
            Err(err) => {
                tracing::error!("cannot clear historical print journal after reconnect: {err}");
                self.journal_ok.store(false, Ordering::Relaxed);
            }
        }
        *self.config.lock().unwrap() = Some(config);
        if let Err(error) = self.set_printers(paired.printers) {
            tracing::error!("paired, but could not save offline printer settings: {error}");
        }
        Ok(())
    }

    pub fn printers(&self) -> serde_json::Value {
        self.printers.lock().unwrap().clone()
    }

    pub fn windows_printers(&self) -> Option<Vec<String>> {
        self.windows_printers
            .lock()
            .unwrap()
            .as_ref()
            .map(|p| printers::names(p))
    }

    pub fn set_windows_printers(&self, list: Vec<WinPrinter>) {
        self.set_windows_printers_with_pnp(list, None);
    }

    pub fn set_windows_printers_with_pnp(
        &self,
        list: Vec<WinPrinter>,
        present_interfaces: Option<&[String]>,
    ) {
        let checked_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        // A user/installer may explicitly bind a queue to its verified USBPRINT
        // path. No guessing by port number, friendly name, or VID/PID.
        // Failure to inspect PnP never implies the USB interface is absent.
        let bindings: HashMap<String, String> =
            std::fs::read(config::folder().join("printer-usb-bindings.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default();
        let health = list
            .iter()
            .map(|p| {
                let present = present_interfaces.and_then(|paths| {
                    bindings.get(&p.name).map(|path| {
                        paths
                            .iter()
                            .any(|interface| interface.eq_ignore_ascii_case(path))
                    })
                });
                printer_state::classify(p, present, None, checked_at)
            })
            .collect();
        if let Some(paths) = present_interfaces {
            self.set_pnp_count(paths.len());
        }
        *self.health.lock().unwrap() = Some(health);
        *self.windows_printers.lock().unwrap() = Some(list);
    }

    pub fn set_printers(&self, printers: serde_json::Value) -> Result<(), String> {
        if !printers.is_array() {
            return Err("Printer settings must be an array".to_string());
        }
        *self.printers.lock().unwrap() = printers.clone();
        std::fs::write(config::folder().join("printers.json"), printers.to_string())
            .map_err(|error| format!("Cannot save offline printer settings: {error}"))
    }

    /// The backend says this agent is no longer paired (it was removed in Settings): forget the key.
    pub fn unpair(&self) {
        config::require_reconnect();
        config::clear();
        *self.windows_printers.lock().unwrap() = None;
        *self.health.lock().unwrap() = None;
        *self.pnp_count.lock().unwrap() = None;
        self.request_printer_refresh();
        if let Err(error) = self.set_printers(serde_json::json!([])) {
            tracing::error!("could not clear offline printer settings: {error}");
        }
        *self.config.lock().unwrap() = None;
        // Keys from a removed restaurant must not suppress slips for a future tenant.
        // Keep the history if disk access fails: fail closed rather than silently erase.
        match std::fs::remove_file(config::folder().join("printed.log")) {
            Ok(()) => self.printed.lock().unwrap().clear(),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                self.printed.lock().unwrap().clear()
            }
            Err(err) => {
                tracing::error!("cannot clear the previous pairing's print journal: {err}");
                self.journal_ok.store(false, Ordering::Relaxed);
            }
        }
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
        let seconds: Vec<u64> = (0..6)
            .map(|attempt| retry_delay(attempt).as_secs())
            .collect();
        assert_eq!(seconds, [2, 5, 10, 30, 60, 60]);
    }

    #[test]
    fn print_identity_includes_order_kind_and_physical_printer() {
        let order: slips::Order = serde_json::from_value(serde_json::json!({
            "dailyToken": "A-1", "type": "takeaway", "items": []
        }))
        .unwrap();
        let header = slips::Header::default();
        let mut slip = Slip {
            order_key: "order-1",
            kind: "kitchen",
            printer: "Kitchen",
            width: 32,
            copies: 1,
            order: &order,
            header: &header,
        };
        let first = Agent::key(&slip);
        assert_eq!(first, "order-1|kitchen|Kitchen");
        slip.kind = "receipt";
        assert_ne!(Agent::key(&slip), first);
        slip.kind = "kitchen";
        slip.printer = "Bar";
        assert_ne!(Agent::key(&slip), first);
        slip.printer = "Kitchen";
        slip.order_key = "order-2";
        assert_ne!(Agent::key(&slip), first);
    }

    #[test]
    fn pairs_only_with_restora_apis() {
        assert_eq!(api_for(None).unwrap(), DEFAULT_API);
        assert_eq!(
            api_for(Some("https://dev.restora.live/api/v1/")).unwrap(),
            "https://dev.restora.live/api/v1"
        );
        assert!(api_for(Some("https://evil.example/api/v1")).is_err());
    }
}
