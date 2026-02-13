//! legion-report — Worker reports completion/error
//!
//! Usage: legion-report <done|error> ["summary text"]
//! POSTs to /legion/orchestrate/report with {"worker_id": N, "status": "done"|"error", "summary": "..."}
//! Gets worker_id from LEGION_WORKER_ID env var (default 0).
//! Also writes result file to /tmp/legion/results/worker-{N}.md

use std::env;
use std::fs;
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

    if args.len() < 2 {
        eprintln!("Usage: legion-report <done|error> [\"summary text\"]");
        process::exit(1);
    }

    let status_str = &args[1];
    if status_str != "done" && status_str != "error" {
        eprintln!("Error: status must be 'done' or 'error', got '{}'", status_str);
        process::exit(1);
    }

    let summary = if args.len() > 2 {
        Some(args[2..].join(" "))
    } else {
        None
    };

    let worker_id = get_worker_id();
    let port = get_orchestrate_port();
    let url = format!("http://127.0.0.1:{}/legion/orchestrate/report", port);

    let mut body = serde_json::json!({
        "worker_id": worker_id,
        "status": status_str,
    });

    if let Some(ref s) = summary {
        body["summary"] = serde_json::Value::String(s.clone());
    }

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
                println!(
                    "Worker {} reported: {}{}",
                    worker_id,
                    status_str,
                    summary
                        .as_ref()
                        .map(|s| format!(" - {}", s))
                        .unwrap_or_default()
                );
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

    // Write result file
    write_result_file(worker_id, status_str, summary.as_deref());
}

fn write_result_file(worker_id: u16, status: &str, summary: Option<&str>) {
    let dir = "/tmp/legion/results";
    if let Err(e) = fs::create_dir_all(dir) {
        eprintln!("Warning: could not create {}: {}", dir, e);
        return;
    }

    let path = format!("{}/worker-{}.md", dir, worker_id);
    let summary_text = summary.unwrap_or("(no summary)");
    let timestamp = chrono_now();

    let content = format!(
        "---\nworker_id: {}\nstatus: {}\ntimestamp: {}\n---\n\n# Worker {} Result\n\n{}\n",
        worker_id, status, timestamp, worker_id, summary_text
    );

    if let Err(e) = fs::write(&path, &content) {
        eprintln!("Warning: could not write {}: {}", path, e);
    } else {
        println!("Result written to {}", path);
    }
}

/// Simple timestamp without pulling in chrono dependency.
fn chrono_now() -> String {
    use std::time::SystemTime;
    match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => format!("{}", d.as_secs()),
        Err(_) => "0".to_string(),
    }
}
