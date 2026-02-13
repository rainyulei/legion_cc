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

    let workers = match parsed.get("workers").and_then(|w| w.as_array()) {
        Some(w) => w,
        None => {
            eprintln!("Error: unexpected response format");
            process::exit(1);
        }
    };

    let mut parts: Vec<String> = Vec::new();

    for w in workers {
        let id = w.get("worker_id").and_then(|v| v.as_u64()).unwrap_or(0);
        let status_str = w
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let badge = match status_str {
            "done" => "OK",
            "working" => "..",
            "pending" => "??",
            "error" => "!!",
            "stopped" => "XX",
            "idle" => "--",
            _ => "??",
        };

        parts.push(format!("W{}[{}]", id, badge));
    }

    println!("{}", parts.join(" "));
}
