use crate::agent::{Agent, Slip, retry_delay};
use crate::{printers, slips};
use serde::Deserialize;
use std::{
    sync::atomic::Ordering,
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

/// How long a slip that will not print is retried before it is reported failed.
const GIVE_UP: Duration = Duration::from_secs(15 * 60);
const POLL: Duration = Duration::from_secs(2);
/// The printers Windows lists are sent again this often (they change when a printer is plugged in).
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

/// A slip that failed to print: when it was first seen, how often it was tried, and when to try next.
struct Trying {
    first: Instant,
    attempts: u32,
    next: Instant,
}

/// Once paired: asks the backend for waiting slips every 2 seconds, prints them and reports each. A slip that cannot be
/// printed (printer off, out of paper) stays waiting at the backend and is tried again; if the agent restarts it is
/// fetched again, and the printed log stops a slip that did print from printing twice.
pub async fn run(agent: Arc<Agent>) {
    let mut trying: HashMap<String, Trying> = HashMap::new();
    let mut reported = Instant::now() - REPORT_PRINTERS;
    loop {
        tokio::time::sleep(POLL).await;
        let Some(config) = agent.config.lock().unwrap().clone() else { continue };
        let url = format!("{}/print-agent", config.api);

        if reported.elapsed() >= REPORT_PRINTERS {
            if let Ok(names) = printers::list() {
                let sent = agent.http.put(format!("{url}/printers")).bearer_auth(&config.token).json(&serde_json::json!({ "version": env!("CARGO_PKG_VERSION"), "printers": names })).send().await;
                if let Ok(response) = sent.map(|r| r.error_for_status()) {
                    if let Ok(answer) = response {
                        if let Ok(body) = answer.json::<serde_json::Value>().await {
                            agent.set_printers(body["printers"].clone());
                        }
                        reported = Instant::now();
                    }
                }
            } else {
                reported = Instant::now();
            }
        }

        let response = match agent.http.get(format!("{url}/jobs")).bearer_auth(&config.token).send().await {
            Ok(response) => response,
            // No internet: nothing is lost, the slips wait at the backend.
            Err(err) => {
                tracing::warn!("cannot reach Restora: {err}");
                agent.online.store(false, Ordering::Relaxed);
                continue;
            }
        };
        agent.online.store(true, Ordering::Relaxed);
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            tracing::warn!("this agent was removed in Settings; pair it again");
            agent.unpair();
            crate::local::open_setup();
            continue;
        }
        let Ok(jobs) = response.json::<Vec<Job>>().await else { continue };

        for mut job in jobs {
            let waiting = trying.get(&job.id);
            if waiting.is_some_and(|t| Instant::now() < t.next) {
                continue;
            }
            if job.kind == "receipt" {
                job.header.logo_file = crate::logo::load(&agent.http, job.header.logo.as_deref()).await;
            }
            let slip = Slip { printer: &job.printer, kind: &job.kind, order_key: &job.order_key, width: job.width, copies: job.copies, order: &job.order, header: &job.header };
            let result = match tokio::task::block_in_place(|| agent.print(&slip)) {
                Ok(_) => {
                    trying.remove(&job.id);
                    serde_json::json!({ "status": "printed" })
                }
                Err(error) => {
                    let t = trying.entry(job.id.clone()).or_insert(Trying { first: Instant::now(), attempts: 0, next: Instant::now() });
                    t.next = Instant::now() + retry_delay(t.attempts);
                    t.attempts += 1;
                    if t.first.elapsed() < GIVE_UP {
                        tracing::warn!("{} did not print (try {}): {error}", job.printer, t.attempts);
                        continue;
                    }
                    trying.remove(&job.id);
                    serde_json::json!({ "status": "failed", "error": error })
                }
            };
            // If this report is lost, the slip is fetched again; a printed one is then only reported, not printed twice.
            let _ = agent.http.post(format!("{url}/jobs/{}", job.id)).bearer_auth(&config.token).json(&result).send().await;
        }
    }
}
