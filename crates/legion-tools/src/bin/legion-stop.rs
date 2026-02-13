//! legion-stop — Stop one or all Workers
//!
//! Usage: legion-stop <worker_id|all>
//! If "all": POSTs to /legion/orchestrate/stop-all
//! Otherwise: POSTs to /legion/orchestrate/stop with {"worker_id": N}

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

    if args.len() < 2 {
        eprintln!("Usage: legion-stop <worker_id|all>");
        process::exit(1);
    }

    let target = &args[1];
    let port = get_orchestrate_port();

    if target == "all" {
        let url = format!("http://127.0.0.1:{}/legion/orchestrate/stop-all", port);

        match ureq::post(&url)
            .content_type("application/json")
            .send(b"{}" as &[u8])
        {
            Ok(response) => {
                let http_status = response.status();
                let body_str = response
                    .into_body()
                    .read_to_string()
                    .unwrap_or_default();

                if http_status.is_success() {
                    println!("All workers stopped");
                } else {
                    eprintln!("Error (HTTP {}): {}", http_status.as_u16(), body_str);
                    process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("Error: failed to connect to orchestrate API: {}", e);
                process::exit(1);
            }
        }
    } else {
        let worker_id: u16 = match target.parse() {
            Ok(id) => id,
            Err(_) => {
                eprintln!("Error: argument must be a worker_id (number) or 'all', got '{}'", target);
                process::exit(1);
            }
        };

        let url = format!("http://127.0.0.1:{}/legion/orchestrate/stop", port);
        let body = serde_json::json!({ "worker_id": worker_id });

        match ureq::post(&url)
            .content_type("application/json")
            .send(body.to_string().as_bytes())
        {
            Ok(response) => {
                let http_status = response.status();
                let body_str = response
                    .into_body()
                    .read_to_string()
                    .unwrap_or_default();

                if http_status.is_success() {
                    println!("Worker {} stopped", worker_id);
                } else {
                    eprintln!("Error (HTTP {}): {}", http_status.as_u16(), body_str);
                    process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("Error: failed to connect to orchestrate API: {}", e);
                process::exit(1);
            }
        }
    }
}
