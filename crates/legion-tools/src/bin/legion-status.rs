//! legion-status — One-line compact summary
//!
//! Usage: legion-status
//! GETs /legion/orchestrate/status
//! Prints: W1[OK] W2[..] W3[--]

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

    let tickets = match parsed.get("tickets").and_then(|t| t.as_array()) {
        Some(t) => t,
        None => {
            eprintln!("Error: unexpected response format");
            process::exit(1);
        }
    };

    if tickets.is_empty() {
        println!("No tickets.");
        return;
    }

    let mut parts: Vec<String> = Vec::new();

    for t in tickets {
        let id = t.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
        let status_str = t
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let worker = t.get("assigned_worker").and_then(|v| v.as_u64());

        let badge = match status_str {
            "done" => "OK",
            "working" => "..",
            "queued" => "??",
            "error" => "!!",
            _ => "??",
        };

        let worker_str = match worker {
            Some(w) => format!("W{}", w),
            None => format!("#{}", id),
        };

        parts.push(format!("{}[{}]", worker_str, badge));
    }

    println!("{}", parts.join(" "));
}
