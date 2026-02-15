//! legion-dispatch — Submit a ticket to the orchestrate queue
//!
//! Usage: legion-dispatch <worker_id> [-t "title"] "ticket text"
//! POSTs to /legion/orchestrate/submit with {"title": "...", "ticket": "...", "team_mode": "tech_lead_team"}
//! (worker_id is kept for CLI compat but ignored by queue)

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
        eprintln!("Usage: legion-dispatch <worker_id> [-t \"title\"] \"ticket text\"");
        process::exit(1);
    }

    let worker_id: u16 = match args[1].parse() {
        Ok(id) => id,
        Err(_) => {
            eprintln!("Error: worker_id must be a number, got '{}'", args[1]);
            process::exit(1);
        }
    };

    // Parse optional --title / -t flag
    let mut title: Option<String> = None;
    let mut ticket_start = 2;
    if args.len() > 3 && (args[2] == "-t" || args[2] == "--title") {
        title = Some(args[3].clone());
        ticket_start = 4;
    }

    if ticket_start >= args.len() {
        eprintln!("Usage: legion-dispatch <worker_id> [-t \"title\"] \"ticket text\"");
        process::exit(1);
    }

    let ticket = args[ticket_start..].join(" ");

    // Auto-generate title from first 40 chars if not provided
    let title = title.unwrap_or_else(|| {
        let t: String = ticket.chars().take(40).collect();
        if ticket.chars().count() > 40 {
            format!("{}...", t)
        } else {
            t
        }
    });

    let port = get_orchestrate_port();
    let url = format!("http://127.0.0.1:{}/legion/orchestrate/submit", port);

    let body = serde_json::json!({
        "title": title,
        "ticket": ticket,
        "team_mode": "tech_lead_team",
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
                println!("Submitted ticket (via worker_id {}): {}", worker_id, ticket);
                // Print ticket ID if returned
                if let Ok(resp) = serde_json::from_str::<serde_json::Value>(&body_str) {
                    if let Some(id) = resp.get("ticket_id").and_then(|v| v.as_str()) {
                        println!("Ticket ID: {}", id);
                    }
                }
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
