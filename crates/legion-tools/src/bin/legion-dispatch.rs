//! legion-dispatch — Leader sends task to Worker
//!
//! Usage: legion-dispatch <worker_id> "ticket text"
//! POSTs to /legion/orchestrate/dispatch with {"worker_id": N, "ticket": "..."}

use std::env;
use std::process;

fn get_orchestrate_port() -> u16 {
    env::var("LEGION_ORCHESTRATE_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20080)
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 3 {
        eprintln!("Usage: legion-dispatch <worker_id> \"ticket text\"");
        process::exit(1);
    }

    let worker_id: u16 = match args[1].parse() {
        Ok(id) => id,
        Err(_) => {
            eprintln!("Error: worker_id must be a number, got '{}'", args[1]);
            process::exit(1);
        }
    };

    let ticket = args[2..].join(" ");

    let port = get_orchestrate_port();
    let url = format!("http://127.0.0.1:{}/legion/orchestrate/dispatch", port);

    let body = serde_json::json!({
        "worker_id": worker_id,
        "ticket": ticket,
    });

    match ureq::post(&url)
        .content_type("application/json")
        .send(body.to_string().as_bytes())
    {
        Ok(response) => {
            let status = response.status();
            let body_str = response
                .into_body()
                .read_to_string()
                .unwrap_or_default();

            if status.is_success() {
                println!("Dispatched to worker {}: {}", worker_id, ticket);
            } else {
                eprintln!("Error (HTTP {}): {}", status.as_u16(), body_str);
                process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Error: failed to connect to orchestrate API: {}", e);
            process::exit(1);
        }
    }
}
