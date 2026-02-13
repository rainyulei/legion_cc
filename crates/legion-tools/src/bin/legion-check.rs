//! legion-check — Leader views all Worker status
//!
//! Usage: legion-check
//! GETs /legion/orchestrate/status and pretty-prints a status board.

use std::env;
use std::process;

fn get_orchestrate_port() -> u16 {
    env::var("LEGION_ORCHESTRATE_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20080)
}

fn main() {
    let port = get_orchestrate_port();
    let url = format!("http://127.0.0.1:{}/legion/orchestrate/status", port);

    let response = match ureq::get(&url).call() {
        Ok(resp) => resp,
        Err(e) => {
            eprintln!("Error: failed to connect to orchestrate API: {}", e);
            process::exit(1);
        }
    };

    let status = response.status();
    let body_str = response
        .into_body()
        .read_to_string()
        .unwrap_or_default();

    if !status.is_success() {
        eprintln!("Error (HTTP {}): {}", status.as_u16(), body_str);
        process::exit(1);
    }

    let parsed: serde_json::Value = match serde_json::from_str(&body_str) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Error: failed to parse response: {}", e);
            process::exit(1);
        }
    };

    let workers = match parsed.get("workers").and_then(|w| w.as_array()) {
        Some(w) => w,
        None => {
            eprintln!("Error: unexpected response format");
            process::exit(1);
        }
    };

    let mut completed = 0u32;
    let total = workers.len() as u32;

    println!("=== Squad Status ===");

    for w in workers {
        let id = w.get("worker_id").and_then(|v| v.as_u64()).unwrap_or(0);
        let status_str = w
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let ticket = w
            .get("ticket")
            .and_then(|v| v.as_str())
            .unwrap_or("-");
        let elapsed = w.get("elapsed_secs").and_then(|v| v.as_u64()).unwrap_or(0);

        let badge = match status_str {
            "done" => {
                completed += 1;
                "OK"
            }
            "working" => "WORKING",
            "pending" => "PENDING",
            "error" => {
                completed += 1;
                "ERROR"
            }
            "stopped" => "STOPPED",
            "idle" => "IDLE",
            other => other,
        };

        let display_ticket = if ticket == "-" || ticket.is_empty() {
            "-".to_string()
        } else if ticket.len() > 40 {
            format!("{}...", &ticket[..37])
        } else {
            ticket.to_string()
        };

        println!(
            "  Worker {}: [{}] \"{}\" ({}s)",
            id, badge, display_ticket, elapsed
        );

        // Show summary if available
        if let Some(summary) = w.get("summary").and_then(|v| v.as_str()) {
            if !summary.is_empty() {
                println!("           Summary: {}", summary);
            }
        }

        // Show result file if available
        let result_path = format!("/tmp/legion/results/worker-{}.md", id);
        if std::path::Path::new(&result_path).exists() {
            println!("           Result: {}", result_path);
        }
    }

    println!("Completed: {}/{}", completed, total);
}
