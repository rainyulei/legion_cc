//! legion-check — View ticket queue status
//!
//! Usage: legion-check
//! GETs /legion/orchestrate/status and pretty-prints the ticket queue.

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

    // Extract queue stats
    let total = parsed.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
    let queued = parsed.get("queued").and_then(|v| v.as_u64()).unwrap_or(0);
    let working = parsed.get("working").and_then(|v| v.as_u64()).unwrap_or(0);
    let done = parsed.get("done").and_then(|v| v.as_u64()).unwrap_or(0);
    let error = parsed.get("error").and_then(|v| v.as_u64()).unwrap_or(0);

    println!("=== Ticket Queue ===");
    println!(
        "Total: {}  |  Queued: {}  |  Working: {}  |  Done: {}  |  Error: {}",
        total, queued, working, done, error
    );
    println!();

    let tickets = match parsed.get("tickets").and_then(|t| t.as_array()) {
        Some(t) => t,
        None => {
            // No tickets array — maybe empty queue
            println!("No tickets.");
            return;
        }
    };

    if tickets.is_empty() {
        println!("No tickets.");
        return;
    }

    // Group tickets by status
    let status_order = ["queued", "working", "done", "error"];

    for &group in &status_order {
        let group_tickets: Vec<&serde_json::Value> = tickets
            .iter()
            .filter(|t| {
                t.get("status")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    == group
            })
            .collect();

        if group_tickets.is_empty() {
            continue;
        }

        let badge = match group {
            "queued" => "QUEUED",
            "working" => "WORKING",
            "done" => "DONE",
            "error" => "ERROR",
            other => other,
        };

        println!("--- {} ({}) ---", badge, group_tickets.len());

        for t in &group_tickets {
            let ticket_id = t
                .get("id")
                .and_then(|v| v.as_u64())
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".to_string());
            let title = t
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let ticket_text = t
                .get("prompt")
                .and_then(|v| v.as_str())
                .unwrap_or("-");
            let worker = t.get("assigned_worker").and_then(|v| v.as_u64());
            let elapsed = t.get("elapsed_secs").and_then(|v| v.as_u64()).unwrap_or(0);

            let display_ticket = if !title.is_empty() {
                title.to_string()
            } else if ticket_text.chars().count() > 50 {
                let truncated: String = ticket_text.chars().take(47).collect();
                format!("{}...", truncated)
            } else {
                ticket_text.to_string()
            };

            let worker_str = match worker {
                Some(w) => format!(" worker={}", w),
                None => String::new(),
            };

            println!(
                "  [{}] \"{}\"{}  ({}s)",
                ticket_id, display_ticket, worker_str, elapsed
            );

            // Show summary if available
            if let Some(summary) = t.get("summary").and_then(|v| v.as_str()) {
                if !summary.is_empty() {
                    println!("         Summary: {}", summary);
                }
            }
        }

        println!();
    }
}
