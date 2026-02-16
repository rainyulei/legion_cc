# GitHub Copilot TUI Device Auth Design

## Problem

When connecting GitHub Copilot in the TUI, the app shows a standard API Key input dialog. GitHub Copilot uses OAuth device flow, not API keys. The `copilot.rs` module already implements the full device flow, but it's only accessible via CLI (`legion copilot login`).

## Design

### New Popup: CopilotAuth

When user selects "GitHub Copilot" in Connect Provider and presses Enter, instead of showing the API Key input, show a CopilotAuth popup that runs the device flow inline.

### UI States

**State 1: Requesting device code**
```
┌ GitHub Copilot Auth [ESC] ───────┐
│                                   │
│  Requesting device code...        │
│                                   │
└───────────────────────────────────┘
```

**State 2: Waiting for user authorization**
```
┌ GitHub Copilot Auth [ESC] ───────┐
│                                   │
│  Please visit:                    │
│  https://github.com/login/device │
│                                   │
│  Enter code: ABCD-1234           │
│                                   │
│  Waiting for authorization...     │
│                                   │
│  [Esc] Cancel                     │
└───────────────────────────────────┘
```

**State 3: Success**
```
┌ GitHub Copilot Auth [ESC] ───────┐
│                                   │
│  Authorized!                      │
│  Models: claude-sonnet-4-5, ...   │
│  Provider saved.                  │
│                                   │
└───────────────────────────────────┘
```

**State 4: Error**
```
┌ GitHub Copilot Auth [ESC] ───────┐
│                                   │
│  Error: Token expired             │
│                                   │
│  [Enter] Retry  [Esc] Cancel     │
└───────────────────────────────────┘
```

### Async Architecture

- `request_device_code()` and `poll_for_access_token()` are async
- Use `tokio::spawn` to run the polling loop in background
- Communicate with TUI via `tokio::sync::mpsc` channel
- Event loop checks channel each tick for status updates

### Channel Messages

```rust
enum CopilotAuthMsg {
    DeviceCode { user_code: String, verification_uri: String },
    Authorized { token: String },
    Error(String),
}
```

### App State

```rust
pub copilot_auth_status: CopilotAuthStatus,
pub copilot_auth_rx: Option<mpsc::UnboundedReceiver<CopilotAuthMsg>>,
pub copilot_user_code: Option<String>,
pub copilot_verification_uri: Option<String>,
pub copilot_auth_error: Option<String>,
```

### Provider Template Change

Add `auth_method` field to `ProviderTemplate`:
```rust
pub auth_method: &'static str, // "api_key" or "device_flow"
```

GitHub Copilot gets `"device_flow"`, all others get `"api_key"`.

### Flow

1. User selects GitHub Copilot → Enter
2. Check `auth_method == "device_flow"`
3. Spawn async task: `request_device_code()` → send `DeviceCode` msg
4. Show popup with user_code + verification_uri
5. Async task polls `poll_for_access_token()` → send `Authorized` msg
6. On authorized: `exchange_copilot_token()` → `fetch_models()` → save provider to DB
7. Show success → auto-close after 2s or on keypress

### Files

- `crates/legion-tui/src/app.rs` — CopilotAuth popup, state fields, auth_method on template
- `crates/legion-tui/src/input.rs` — route Copilot to device flow, handle CopilotAuth keys
- `crates/legion-tui/src/ui.rs` — draw CopilotAuth popup
- `crates/legion-tui/src/lib.rs` — check auth channel in event loop
