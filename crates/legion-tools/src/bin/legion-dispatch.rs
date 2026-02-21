//! legion-dispatch — Submit a ticket to the orchestrate queue
//!
//! Usage: legion-dispatch <worker_id> [-t "title"] [-c "context"] [-k "criteria"] [--after 1,3] [--team <team_name>] [--plan "structure plan"] "ticket text"
//! POSTs to /legion/orchestrate/submit with structured fields.
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
        eprintln!("Usage: legion-dispatch <worker_id> [-t \"title\"] [-c \"context\"] [-k \"criteria\"] [--after 1,3] [--team <team_name>] [--plan \"structure plan\"] \"ticket text\"");
        process::exit(1);
    }

    let worker_id: u16 = match args[1].parse() {
        Ok(id) => id,
        Err(_) => {
            eprintln!("Error: worker_id must be a number, got '{}'", args[1]);
            process::exit(1);
        }
    };

    // Parse optional flags
    let mut title: Option<String> = None;
    let mut context: Option<String> = None;
    let mut criteria: Option<String> = None;
    let mut after: Option<String> = None;
    let mut team: Option<String> = None;
    let mut plan: Option<String> = None;
    let mut positional: Vec<String> = Vec::new();

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "-t" | "--title" => {
                i += 1;
                if i < args.len() { title = Some(args[i].clone()); }
            }
            "-c" | "--context" => {
                i += 1;
                if i < args.len() { context = Some(args[i].clone()); }
            }
            "-k" | "--criteria" => {
                i += 1;
                if i < args.len() { criteria = Some(args[i].clone()); }
            }
            "-a" | "--after" => {
                i += 1;
                if i < args.len() { after = Some(args[i].clone()); }
            }
            "--team" => {
                i += 1;
                if i < args.len() { team = Some(args[i].clone()); }
            }
            "-p" | "--plan" => {
                i += 1;
                if i < args.len() { plan = Some(args[i].clone()); }
            }
            _ => positional.push(args[i].clone()),
        }
        i += 1;
    }

    if positional.is_empty() {
        eprintln!("Usage: legion-dispatch <worker_id> -t \"title\" -c \"context\" -k \"criteria\" [--after 1,3] [--team <team_name>] \"ticket text\"");
        process::exit(1);
    }

    if context.is_none() {
        eprintln!("Error: -c/--context is required. Provide working directory, language, related files, etc.");
        eprintln!("Example: legion-dispatch 1 -t \"Add login\" -c \"Rust project in ./backend, uses axum\" -k \"tests pass\" --after 1,3 \"implement login\"");
        process::exit(1);
    }

    if criteria.is_none() {
        eprintln!("Error: -k/--criteria is required. Provide specific, testable success conditions.");
        eprintln!("Example: legion-dispatch 1 -t \"Add login\" -c \"Rust project in ./backend\" -k \"POST /login returns JWT, cargo test passes\" --after 1,2 \"implement login\"");
        process::exit(1);
    }

    let ticket = positional.join(" ");

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

    let mut body = serde_json::json!({
        "title": title,
        "ticket": ticket,
        "team_mode": team.as_deref().unwrap_or("tech_lead_team"),
    });

    if let Some(ctx) = &context {
        body["context"] = serde_json::Value::String(ctx.clone());
    }
    if let Some(crit) = &criteria {
        body["criteria"] = serde_json::Value::String(crit.clone());
    }
    if let Some(after_str) = &after {
        let blocked_by: Vec<u64> = after_str.split(',')
            .filter_map(|s| s.trim().parse::<u64>().ok())
            .collect();
        if !blocked_by.is_empty() {
            body["blocked_by"] = serde_json::json!(blocked_by);
        }
    }
    if let Some(plan_str) = &plan {
        body["structure_plan"] = serde_json::Value::String(plan_str.clone());
    }

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
                if let Some(ref t) = team {
                    println!("Team: {}", t);
                }
                if let Ok(resp) = serde_json::from_str::<serde_json::Value>(&body_str) {
                    if let Some(id) = resp.get("ticket_id") {
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
