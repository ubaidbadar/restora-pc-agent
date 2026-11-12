use crate::agent::{Agent, Slip, retry_delay};
use crate::config::Config;
use crate::{printers, slips};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, atomic::Ordering},
    time::{Duration, Instant},
};

/// Give up a job after persistent spooler errors; the server then shows it as failed.
const GIVE_UP: Duration = Duration::from_secs(15 * 60);
const POLL: Duration = Duration::from_secs(2);
const REPORT_PRINTERS: Duration = Duration::from_secs(5 * 60);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Job {
    #[serde(rename = "_id")]
    id: String,
    printer: String,
    kind: String,
    order_key: String,
    width: usize,
    copies: u32,
    order: slips::Order,
    #[serde(default)]
    header: slips::Header,
}

/// Retry state lives across polls; a job that disappears from the server
/// is never retried by a long-running detached retry loop.
struct Trying {
    first: Instant,
    attempts: u32,
    next: Instant,
}

/// Preserve server FIFO independently for every physical printer.
/// A blocked kitchen printer must not stop a receipt on another printer.
fn oldest_per_printer(jobs: Vec<Job>) -> Vec<Job> {
    let mut seen = HashSet::new();
    jobs.into_iter()
        .filter(|job| seen.insert(job.printer.clone()))
        .collect()
}

/// One attempt only. Errors are scheduled for a later poll, so cancellation
/// on the backend prevents future retries. The per-printer lock is in Agent.
async fn attempt(
    agent: Arc<Agent>,
    config: Config,
    url: String,
    mut job: Job,
    trying: Arc<Mutex<HashMap<String, Trying>>>,
) {
    let active_pairing = |agent: &Agent| {
        agent
            .config
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|current| current.token == config.token && current.api == config.api)
    };
    if !active_pairing(&agent) {
        return;
    }
    if job.kind == "receipt" {
        job.header.logo_file = crate::logo::load(job.header.logo.as_deref()).await;
    }

    let job_id = job.id.clone();
    let printer = job.printer.clone();
    let print_agent = agent.clone();
    let expected_token = config.token.clone();
    let expected_api = config.api.clone();
    let result = tokio::task::spawn_blocking(move || {
        let same_pairing = print_agent
            .config
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|current| current.token == expected_token && current.api == expected_api);
        if !same_pairing {
            return Err("Agent connection changed while the print job was waiting".to_string());
        }
        let slip = Slip {
            printer: &job.printer,
            kind: &job.kind,
            order_key: &job.order_key,
            width: job.width,
            copies: job.copies,
            order: &job.order,
            header: &job.header,
        };
        print_agent.print(&slip)
    })
    .await
    .unwrap_or_else(|error| Err(error.to_string()));

    if !active_pairing(&agent) {
        // The owner switched or removed the agent. Never report this job with a stale token.
        return;
    }
    let report = match result {
        Ok(_) => {
            trying.lock().unwrap().remove(&job_id);
            serde_json::json!({ "status": "printed" })
        }
        Err(error) => {
            let (expired, attempt) = {
                let mut retries = trying.lock().unwrap();
                let item = retries.entry(job_id.clone()).or_insert_with(|| Trying {
                    first: Instant::now(),
                    attempts: 0,
                    next: Instant::now(),
                });
                item.next = Instant::now() + retry_delay(item.attempts);
                item.attempts += 1;
                let expired = item.first.elapsed() >= GIVE_UP;
                let attempt = item.attempts;
                if expired {
                    retries.remove(&job_id);
                }
                (expired, attempt)
            };
            if !expired {
                tracing::warn!("{printer} failed print attempt {attempt}: {error}");
                return;
            }
            serde_json::json!({ "status": "failed", "error": error })
        }
    };

    // A lost acknowledgment is harmless: a durable local key suppresses the
    // next submission, while the backend accepts repeat result reports.
    if let Err(error) = agent
        .http
        .post(format!("{url}/jobs/{job_id}"))
        .bearer_auth(&config.token)
        .json(&report)
        .send()
        .await
    {
        tracing::warn!("cannot report print job {job_id} to Restora: {error}");
    }
}

/// Polling and Windows printer reporting remain responsive while one physical
/// printer/driver stalls. At most one oldest pending job per printer is active.
pub async fn run(agent: Arc<Agent>) {
    let trying = Arc::new(Mutex::new(HashMap::<String, Trying>::new()));
    let active_printers = Arc::new(Mutex::new(HashSet::<String>::new()));
    let mut reported = Instant::now() - REPORT_PRINTERS;
    let mut printer_revision = String::new();

    loop {
        tokio::time::sleep(POLL).await;
        let Some(config) = agent.config.lock().unwrap().clone() else {
            continue;
        };
        let url = format!("{}/print-agent", config.api);

        if reported.elapsed() >= REPORT_PRINTERS {
            if let Ok(names) = printers::list() {
                let result = agent
                    .http
                    .put(format!("{url}/printers"))
                    .bearer_auth(&config.token)
                    .json(&serde_json::json!({
                        "version": env!("CARGO_PKG_VERSION"), "printers": names,
                    }))
                    .send()
                    .await;
                if let Ok(Ok(answer)) = result.map(|response| response.error_for_status())
                    && let Ok(body) = answer.json::<serde_json::Value>().await
                {
                    if let Err(error) = agent.set_printers(body["printers"].clone()) {
                        tracing::warn!("{error}");
                    } else {
                        reported = Instant::now();
                    }
                }
            } else {
                reported = Instant::now();
            }
        }

        let response = match agent
            .http
            .get(format!("{url}/jobs"))
            .bearer_auth(&config.token)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!("cannot reach Restora: {error}");
                agent.online.store(false, Ordering::Relaxed);
                continue;
            }
        };
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            tracing::warn!("agent authorization removed; reconnect in Restora");
            agent.online.store(false, Ordering::Relaxed);
            agent.unpair();
            crate::local::open_setup();
            continue;
        }
        if !response.status().is_success() {
            agent.online.store(false, Ordering::Relaxed);
            tracing::warn!("cannot fetch print jobs: HTTP {}", response.status());
            continue;
        }
        agent.online.store(true, Ordering::Relaxed);
        // Existing backend jobs response stays an array; only its header announces
        // edited printer assignments. GET the settings only when the revision changes.
        let revision = response
            .headers()
            .get("X-Restora-Printer-Revision")
            .and_then(|header| header.to_str().ok())
            .unwrap_or("")
            .to_string();
        if !revision.is_empty()
            && revision != printer_revision
            && let Ok(answer) = agent
                .http
                .get(format!("{url}/printers"))
                .bearer_auth(&config.token)
                .send()
                .await
            && let Ok(answer) = answer.error_for_status()
            && let Ok(body) = answer.json::<serde_json::Value>().await
            && body["printers"].is_array()
        {
            if let Err(error) = agent.set_printers(body["printers"].clone()) {
                tracing::warn!("{error}");
            } else {
                printer_revision = revision;
            }
        }
        let Ok(jobs) = response.json::<Vec<Job>>().await else {
            continue;
        };

        for job in oldest_per_printer(jobs) {
            let id = job.id.clone();
            if trying
                .lock()
                .unwrap()
                .get(&id)
                .is_some_and(|item| Instant::now() < item.next)
            {
                continue;
            }
            let printer = job.printer.clone();
            if !active_printers.lock().unwrap().insert(printer.clone()) {
                continue;
            }
            let job_agent = agent.clone();
            let job_config = config.clone();
            let job_url = url.clone();
            let job_retries = trying.clone();
            let activity = active_printers.clone();
            tokio::spawn(async move {
                attempt(job_agent, job_config, job_url, job, job_retries).await;
                activity.lock().unwrap().remove(&printer);
            });
        }
        // Avoid retaining old retry metadata for jobs the server has removed.
        trying
            .lock()
            .unwrap()
            .retain(|_, retry| retry.first.elapsed() < GIVE_UP * 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatches_oldest_job_for_each_printer_without_starving_other_printers() {
        let order = serde_json::json!({"dailyToken":"1","type":"takeaway","items":[]});
        let jobs: Vec<Job> = ["Kitchen", "Kitchen", "Receipt", "Bar", "Receipt"]
            .iter()
            .enumerate()
            .map(|(index, printer)| {
                serde_json::from_value(serde_json::json!({
                    "_id": format!("job-{index}"), "printer": printer,
                    "kind": "kitchen", "orderKey": format!("order-{index}"),
                    "width":32, "copies":1, "order": order,
                }))
                .unwrap()
            })
            .collect();
        let chosen = oldest_per_printer(jobs);
        assert_eq!(
            chosen.iter().map(|job| job.id.as_str()).collect::<Vec<_>>(),
            vec!["job-0", "job-2", "job-3"]
        );
    }
}
