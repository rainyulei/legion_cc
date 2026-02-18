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

            let blocked_by: Vec<u64> = t.get("blocked_by")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_u64()).collect())
                .unwrap_or_default();

            let merge_status = t.get("merge_status")
                .and_then(|v| v.as_str())
                .unwrap_or("pending");

            let display_ticket = if !title.is_empty() {
                title.to_string()
            } else if ticket_text.chars().count() > 50 {
                let truncated: String = ticket_text.chars().take(47).collect();
                format!("{}...", truncated)
            } else {
                ticket_text.to_string()
            };

            let mut extras = Vec::new();
            if let Some(w) = worker {
                extras.push(format!("worker={}", w));
            }
            if !blocked_by.is_empty() {
                let dep_strs: Vec<String> = blocked_by.iter().map(|dep_id| {
                    let dep_status = tickets.iter()
                        .find(|dt| dt.get("id").and_then(|v| v.as_u64()) == Some(*dep_id))
                        .and_then(|dt| dt.get("status").and_then(|v| v.as_str()))
                        .unwrap_or("?");
                    format!("#{} {}", dep_id, dep_status)
                }).collect();
                extras.push(format!("after: {}", dep_strs.join(" ")));
            }
            if merge_status != "pending" {
                extras.push(format!("[{}]", merge_status));
            }

            let extras_str = if extras.is_empty() {
                String::new()
            } else {
                format!("  {}", extras.join("  "))
            };

            println!(
                "  [{}] \"{}\"{}  ({}s)",
                ticket_id, display_ticket, extras_str, elapsed
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
