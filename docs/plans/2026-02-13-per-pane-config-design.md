# Per-Pane Provider/Model Configuration Design

## Goal

Each TUI pane (Leader + Workers) gets independent provider and model configuration, managed through a matrix view UI with column-switching interaction.

## Context

Currently, provider and model are app-wide globals. All panes share the same provider/model. The user wants:
- Each pane can use a different provider AND different model
- Matrix view UI showing all panes' configurations at a glance
- Batch assignment capability (All Workers, All Panes)
- Unified UI for both single-pane and squad modes

Reference: cc-switch's per-app proxy config pattern, where each app (Claude/Codex/Gemini/OpenCode) has independent provider configuration.

## Data Model

### Pane struct changes

```rust
pub struct Pane {
    pub pty: Option<PtyHandle>,
    pub proxy_port: u16,
    pub control_port: u16,
    pub label: String,
    pub current_provider: Option<usize>,  // NEW: index into App::providers
    pub current_model: Option<String>,    // NEW: model name
}
```

`App::current_provider` and `App::current_model` remain as global defaults (inherited by new panes).

### State machine

```
PopupMenu enum:
  Main
  Matrix          // NEW: replaces direct Provider/Model entries
  Provider        // Reused: now knows its target
  Model           // Reused: now knows its target

MatrixCol enum:
  Provider
  Model

ModelTarget enum:
  Pane(usize)
  AllWorkers
  AllPanes
```

New App fields:
```rust
pub matrix_row: usize,          // selected row in matrix
pub matrix_col: MatrixCol,      // selected column (Provider or Model)
pub model_target: Option<ModelTarget>,  // what pane(s) to apply selection to
```

## Matrix View UI

### Layout

```
+-- Configuration [ESC] ----------------------------------------+
|                                                                |
|  Pane          Provider          Model                         |
|  ------        ----------        --------------                |
|  Leader       [Copilot *]       [claude-opus-4-6]              |
|  Worker 1     [Copilot *]       [claude-sonnet-4-5]            |
|  Worker 2     [DeepSeek]        [deepseek-v3]                  |
|  ------------------------------------------------              |
|  All Workers  [         ]       [                ]             |
|  All Panes    [         ]       [                ]             |
|                                                                |
|  Tab: Column  | j/k: Row  | Enter: Select  | Esc: Back        |
+----------------------------------------------------------------+
```

### Interaction

- **j/k** (or Up/Down): Navigate rows (panes + All Workers + All Panes)
- **Tab** (or Left/Right): Switch between Provider and Model columns
- **Enter**: Open selection list for current column
  - Provider column -> Provider list popup
  - Model column -> Model list popup (filtered by that pane's provider)
- **Esc**: Return to Main Menu

### Visual highlighting

- Current row: entire row highlighted
- Current column: column value in accent color (Yellow=Provider, Magenta=Model)
- Intersection (row+col): brightest, indicates what Enter will edit

### Single pane mode

Same matrix view, just one row:
```
+-- Configuration [ESC] -----------------------+
|                                               |
|  Pane          Provider      Model            |
|  ------        ---------     ----------       |
|  Claude Code  [Copilot *]   [opus-4-6]       |
|                                               |
|  Tab: Column | Enter: Select | Esc: Back      |
+-----------------------------------------------+
```

## Sub-menu Flow

### From Matrix -> Provider selection

```
+-- Select Provider for [target] [ESC] --+
|                                         |
|  > Copilot *                            |
|    DeepSeek                             |
|    OpenRouter                           |
|                                         |
+-----------------------------------------+
```

- Title shows target: "Leader", "Worker 1", "All Workers", etc.
- Current provider marked with bullet
- After selection: return to Matrix (not Main Menu)
- Switching provider resets that pane's model to provider's first model

### From Matrix -> Model selection

```
+-- Select Model for [target] [ESC] -----+
|                                         |
|  > claude-opus-4-6 *                    |
|    claude-sonnet-4-5                    |
|    claude-haiku-4-5                     |
|                                         |
+-----------------------------------------+
```

- Shows models from the target pane's current provider
- For "All Workers"/"All Panes" with mixed providers: shows models from the globally selected provider
- After selection: return to Matrix

## Main Menu Simplification

Before:
```
> Provider    [Copilot *]
  Model       [opus-4-6]
  ----------
  Quit
```

After:
```
> Config      [3 panes]     (or [1 pane] in single mode)
  ----------
  Quit
```

Provider and Model entries merged into single "Config" entry since the matrix view handles both.

## Control API Updates

Each pane's proxy updated independently:
```
POST http://127.0.0.1:{pane.control_port}/legion/config
{
    "target_url": provider.base_url,
    "api_format": provider.api_format,
    "api_key": provider.api_key,
    "model": pane.current_model
}
```

For batch targets (All Workers/All Panes): iterate and send to each affected pane.

## Header Display

Shows focused pane's provider/model:
```
Legion v0.1.0   [Copilot -> claude-opus-4-6] *
```

Header updates when focus changes between panes (Tab in normal mode).

## Pane Title Display

Squad mode pane border titles show model:
```
+-- Leader | claude-opus-4-6 ------+
```

Single mode:
```
+-- Claude Code ---+
```

## Edge Cases

1. **No providers in DB**: Matrix shows empty state message
2. **Provider has no models**: Model column shows "--", Enter on Model column shows "No models available"
3. **Mixed providers for batch model select**: Use the globally selected provider's model list, or show error if no global provider
4. **Pane's provider deleted**: Falls back to showing "--" and requiring re-selection
