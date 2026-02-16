# GitHub Copilot TUI Device Auth Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** When connecting GitHub Copilot in the TUI, run OAuth device flow inline instead of showing API Key input.

**Architecture:** Add `auth_method` to `ProviderTemplate` to distinguish device-flow providers. Route Copilot to a new `CopilotAuth` popup that spawns an async task for device flow, communicates via `tokio::sync::mpsc` channel, and saves the provider on success.

**Tech Stack:** Rust, tokio (async spawn + mpsc), legion-core::copilot (existing device flow), ratatui (TUI popup)

---

### Task 1: Add auth_method to ProviderTemplate

**Files:**
- Modify: `crates/legion-tui/src/app.rs:94-152`

**Step 1: Add field to ProviderTemplate struct**

```rust
pub struct ProviderTemplate {
    pub id: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    pub api_format: &'static str,
    pub models: &'static [&'static str],
    pub env_var: &'static str,
    pub auth_method: &'static str, // "api_key" or "device_flow"
}
```

**Step 2: Update all template entries**

Add `auth_method: "api_key"` to Anthropic, OpenAI, OpenRouter, Google Gemini, DeepSeek.
Add `auth_method: "device_flow"` to GitHub Copilot.

**Step 3: Build and test**

Run: `cargo build && cargo test`

**Step 4: Commit**

```
feat(tui): add auth_method field to ProviderTemplate
```

---

### Task 2: Add CopilotAuth popup state and channel types

**Files:**
- Modify: `crates/legion-tui/src/app.rs` (PopupMenu enum, App struct)

**Step 1: Add PopupMenu variant**

```rust
CopilotAuth,
```

**Step 2: Add CopilotAuthStatus enum and App fields**

Add near the top of app.rs (after imports):

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopilotAuthStatus {
    RequestingCode,
    WaitingForAuth,
    Exchanging,
    Success,
    Error,
}
```

Add to App struct:

```rust
pub copilot_auth_status: CopilotAuthStatus,
pub copilot_auth_rx: Option<tokio::sync::mpsc::UnboundedReceiver<CopilotAuthMsg>>,
pub copilot_user_code: Option<String>,
pub copilot_verification_uri: Option<String>,
pub copilot_auth_error: Option<String>,
pub copilot_models_result: Option<Vec<String>>,
```

Add the message enum (in app.rs or a sub-module):

```rust
pub enum CopilotAuthMsg {
    DeviceCode { user_code: String, verification_uri: String },
    Authorized,
    SetupComplete { models: Vec<String> },
    Error(String),
}
```

**Step 3: Initialize in App::new()**

```rust
copilot_auth_status: CopilotAuthStatus::RequestingCode,
copilot_auth_rx: None,
copilot_user_code: None,
copilot_verification_uri: None,
copilot_auth_error: None,
copilot_models_result: None,
```

**Step 4: Build and test**

Run: `cargo build && cargo test`

**Step 5: Commit**

```
feat(tui): add CopilotAuth popup state and message types
```

---

### Task 3: Route Copilot to device flow in Connect Provider

**Files:**
- Modify: `crates/legion-tui/src/input.rs:547-555` (handle_connect_provider_keys Enter branch)

**Step 1: Check auth_method before routing**

Replace the `KeyCode::Enter` arm in `handle_connect_provider_keys`:

```rust
KeyCode::Enter => {
    let tmpl = &crate::app::PROVIDER_TEMPLATES[app.connect_provider_index];
    if tmpl.auth_method == "device_flow" {
        // Start Copilot device auth flow
        app.copilot_auth_status = crate::app::CopilotAuthStatus::RequestingCode;
        app.copilot_user_code = None;
        app.copilot_verification_uri = None;
        app.copilot_auth_error = None;
        app.copilot_models_result = None;

        // Spawn async auth task
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        app.copilot_auth_rx = Some(rx);

        let provider_index = app.connect_provider_index;
        tokio::spawn(async move {
            // Step 1: Request device code
            match legion_core::copilot::request_device_code().await {
                Ok(dc) => {
                    let _ = tx.send(crate::app::CopilotAuthMsg::DeviceCode {
                        user_code: dc.user_code,
                        verification_uri: dc.verification_uri,
                    });
                    // Step 2: Poll for access token
                    match legion_core::copilot::poll_for_access_token(&dc.device_code, dc.interval).await {
                        Ok(github_token) => {
                            let _ = tx.send(crate::app::CopilotAuthMsg::Authorized);
                            // Step 3: Full setup (exchange + fetch models)
                            match legion_core::copilot::full_setup(&github_token).await {
                                Ok((_info, models)) => {
                                    // Save the gho token (used for re-exchange later)
                                    // Provider saving happens in the event loop handler
                                    let tmpl = &crate::app::PROVIDER_TEMPLATES[provider_index];
                                    if let Ok(repo) = legion_db::open_db() {
                                        let now = std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_secs() as i64;
                                        let provider = legion_db::Provider {
                                            id: tmpl.id.to_string(),
                                            name: tmpl.name.to_string(),
                                            base_url: tmpl.base_url.to_string(),
                                            api_key: Some(github_token),
                                            api_format: tmpl.api_format.to_string(),
                                            models: Some(models.clone()),
                                            is_default: false,
                                            created_at: now,
                                        };
                                        let _ = repo.upsert_provider(&provider);
                                    }
                                    let _ = tx.send(crate::app::CopilotAuthMsg::SetupComplete { models });
                                }
                                Err(e) => {
                                    let _ = tx.send(crate::app::CopilotAuthMsg::Error(format!("Setup failed: {}", e)));
                                }
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(crate::app::CopilotAuthMsg::Error(format!("{}", e)));
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(crate::app::CopilotAuthMsg::Error(format!("{}", e)));
                }
            }
        });

        app.mode = AppMode::Popup(PopupMenu::CopilotAuth);
    } else {
        // Standard API key input
        let tmpl = &crate::app::PROVIDER_TEMPLATES[app.connect_provider_index];
        app.api_key_input = std::env::var(tmpl.env_var).unwrap_or_default();
        app.mode = AppMode::Popup(PopupMenu::ProviderApiKeyInput);
    }
}
```

**Step 2: Build and test**

Run: `cargo build && cargo test`

**Step 3: Commit**

```
feat(tui): route Copilot to device auth flow instead of API key input
```

---

### Task 4: Handle CopilotAuth channel messages in event loop

**Files:**
- Modify: `crates/legion-tui/src/lib.rs` (run_event_loop)

**Step 1: Add channel polling in event loop**

In `run_event_loop`, after the branch check block and before `terminal.draw()`, add:

```rust
    // Check Copilot auth channel
    if let Some(ref mut rx) = app.copilot_auth_rx {
        while let Ok(msg) = rx.try_recv() {
            match msg {
                app::CopilotAuthMsg::DeviceCode { user_code, verification_uri } => {
                    app.copilot_user_code = Some(user_code);
                    app.copilot_verification_uri = Some(verification_uri);
                    app.copilot_auth_status = app::CopilotAuthStatus::WaitingForAuth;
                }
                app::CopilotAuthMsg::Authorized => {
                    app.copilot_auth_status = app::CopilotAuthStatus::Exchanging;
                }
                app::CopilotAuthMsg::SetupComplete { models } => {
                    app.copilot_models_result = Some(models);
                    app.copilot_auth_status = app::CopilotAuthStatus::Success;
                    // Reload providers from DB
                    app.load_from_db();
                }
                app::CopilotAuthMsg::Error(e) => {
                    app.copilot_auth_error = Some(e);
                    app.copilot_auth_status = app::CopilotAuthStatus::Error;
                }
            }
        }
    }
```

**Step 2: Build and test**

Run: `cargo build && cargo test`

**Step 3: Commit**

```
feat(tui): handle Copilot auth messages in event loop
```

---

### Task 5: Add CopilotAuth key handler

**Files:**
- Modify: `crates/legion-tui/src/input.rs`

**Step 1: Add dispatch entry**

In the popup mode match (where other popups are dispatched):

```rust
AppMode::Popup(PopupMenu::CopilotAuth) => handle_copilot_auth_keys(app, key),
```

**Step 2: Add handler function**

```rust
fn handle_copilot_auth_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            // Cancel — drop the channel receiver to signal the spawned task
            app.copilot_auth_rx = None;
            app.copilot_auth_status = crate::app::CopilotAuthStatus::RequestingCode;
            app.mode = AppMode::Popup(PopupMenu::ConnectProvider);
        }
        KeyCode::Enter => {
            match app.copilot_auth_status {
                crate::app::CopilotAuthStatus::Success => {
                    // Done — go back to connect provider list
                    app.copilot_auth_rx = None;
                    app.mode = AppMode::Popup(PopupMenu::ConnectProvider);
                }
                crate::app::CopilotAuthStatus::Error => {
                    // Retry — go back to connect provider, user can press Enter again
                    app.copilot_auth_rx = None;
                    app.copilot_auth_status = crate::app::CopilotAuthStatus::RequestingCode;
                    app.mode = AppMode::Popup(PopupMenu::ConnectProvider);
                }
                _ => {}
            }
        }
        _ => {}
    }
}
```

**Step 3: Build and test**

Run: `cargo build && cargo test`

**Step 4: Commit**

```
feat(tui): add CopilotAuth key handler with cancel/retry
```

---

### Task 6: Draw CopilotAuth popup

**Files:**
- Modify: `crates/legion-tui/src/ui.rs`

**Step 1: Add popup dispatch and size**

In the popup size match, add:
```rust
PopupMenu::CopilotAuth => (60, 35),
```

In the popup draw dispatch, add:
```rust
PopupMenu::CopilotAuth => draw_copilot_auth(frame, app, area),
```

In the footer hints, add an entry for CopilotAuth.

**Step 2: Add draw function**

```rust
fn draw_copilot_auth(frame: &mut Frame, app: &App, area: Rect) {
    use crate::app::CopilotAuthStatus;

    let block = Block::default()
        .title(" GitHub Copilot Auth [ESC] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::DarkGray));

    let mut items = vec![];

    match app.copilot_auth_status {
        CopilotAuthStatus::RequestingCode => {
            items.push(ListItem::new(Line::from(Span::styled(
                "  Requesting device code...",
                Style::default().fg(Color::Yellow),
            ))));
        }
        CopilotAuthStatus::WaitingForAuth => {
            if let Some(ref uri) = app.copilot_verification_uri {
                items.push(ListItem::new(Line::from(Span::styled(
                    "  Please visit:",
                    Style::default().fg(Color::White),
                ))));
                items.push(ListItem::new(Line::from(Span::styled(
                    format!("  {}", uri),
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ))));
            }
            items.push(ListItem::new(""));
            if let Some(ref code) = app.copilot_user_code {
                items.push(ListItem::new(Line::from(Span::styled(
                    "  Enter code:",
                    Style::default().fg(Color::White),
                ))));
                items.push(ListItem::new(Line::from(Span::styled(
                    format!("  {}", code),
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                ))));
            }
            items.push(ListItem::new(""));
            items.push(ListItem::new(Line::from(Span::styled(
                "  Waiting for authorization...",
                Style::default().fg(Color::Yellow),
            ))));
        }
        CopilotAuthStatus::Exchanging => {
            items.push(ListItem::new(Line::from(Span::styled(
                "  Authorized! Exchanging token...",
                Style::default().fg(Color::Green),
            ))));
        }
        CopilotAuthStatus::Success => {
            items.push(ListItem::new(Line::from(Span::styled(
                "  \u{2713} GitHub Copilot connected!",
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            ))));
            if let Some(ref models) = app.copilot_models_result {
                items.push(ListItem::new(""));
                let model_str = if models.len() > 5 {
                    format!("  Models: {} (+{} more)", models[..5].join(", "), models.len() - 5)
                } else {
                    format!("  Models: {}", models.join(", "))
                };
                items.push(ListItem::new(Line::from(Span::styled(
                    model_str,
                    Style::default().fg(Color::Gray),
                ))));
            }
            items.push(ListItem::new(""));
            items.push(ListItem::new(Line::from(Span::styled(
                "  [Enter] Done",
                Style::default().fg(Color::Gray),
            ))));
        }
        CopilotAuthStatus::Error => {
            items.push(ListItem::new(Line::from(Span::styled(
                "  Error:",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ))));
            if let Some(ref err) = app.copilot_auth_error {
                items.push(ListItem::new(Line::from(Span::styled(
                    format!("  {}", err),
                    Style::default().fg(Color::Red),
                ))));
            }
            items.push(ListItem::new(""));
            items.push(ListItem::new(Line::from(Span::styled(
                "  [Enter] Retry  [Esc] Cancel",
                Style::default().fg(Color::Gray),
            ))));
        }
    }

    frame.render_widget(List::new(items).block(block), area);
}
```

**Step 3: Build and test**

Run: `cargo build && cargo test`

**Step 4: Commit**

```
feat(tui): draw CopilotAuth popup with status-based rendering
```
