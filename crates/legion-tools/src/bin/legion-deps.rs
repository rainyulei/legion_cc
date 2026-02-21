//! legion-deps — Query upstream dependency info for the current ticket
//!
//! Usage: legion-deps [ticket_id]
//! GETs /legion/orchestrate/deps?ticket_id=N
//! If ticket_id is not provided, uses the current worker's active ticket.

use std::env;
use std::process;

fn get_orchestrate_port() -> u16 {
    env::var("LEGION_ORCHESTRATE_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20080)
}

fn get_worker_id() -> u16 {
    env::var("LEGION_WORKER_ID")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let port = get_orchestrate_port();

    // If ticket_id provided as argument, use it directly
    let ticket_id: Option<usize> = args.get(1).and_then(|a| a.parse().ok());

    // If no ticket_id, find the current worker's active ticket from status
    let ticket_id = match ticket_id {
        Some(id) => id,
        None => {
            let worker_id = get_worker_id();
            let status_url = format!("http://127.0.0.1:{}/legion/orchestrate/status", port);
            let resp = match ureq::get(&status_url).call() {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Error: failed to connect to orchestrate API: {}", e);
                    process::exit(1);
                }
            };
            let body: String = resp.into_body().read_to_string().unwrap_or_default();
            let parsed: serde_json::Value = match serde_json::from_str(&body) {
                Ok(v) => v,
                Err(_) => {
                    eprintln!("Error: failed to parse status response");
                    process::exit(1);
                }
            };
            let tickets = parsed.get("tickets").and_then(|t| t.as_array());
            match tickets {
                Some(ts) => {
                    // Find working ticket assigned to this worker
                    match ts.iter().find(|t| {
                        t.get("assigned_worker").and_then(|w| w.as_u64()) == Some(worker_id as u64)
                            && t.get("status").and_then(|s| s.as_str()) == Some("working")
                    }) {
                        Some(t) => t.get("id").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                        None => {
                            eprintln!("Error: no active ticket found for worker {}", worker_id);
                            process::exit(1);
                        }
                    }
                }
                None => {
                    eprintln!("Error: no tickets in status response");
                    process::exit(1);
                }
            }
        }
    };

    // Query deps endpoint
    let deps_url = format!(
        "http://127.0.0.1:{}/legion/orchestrate/deps?ticket_id={}",
        port, ticket_id
    );

    let resp = match ureq::get(&deps_url).call() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error: failed to query deps: {}", e);
            process::exit(1);
        }
    };

    let status = resp.status();
    let body: String = resp.into_body().read_to_string().unwrap_or_default();

    if !status.is_success() {
        eprintln!("Error (HTTP {}): {}", status.as_u16(), body);
        process::exit(1);
    }

    // Pretty print the JSON
    match serde_json::from_str::<serde_json::Value>(&body) {
        Ok(v) => println!("{}", serde_json::to_string_pretty(&v).unwrap_or(body)),
        Err(_) => println!("{}", body),
    }
}
