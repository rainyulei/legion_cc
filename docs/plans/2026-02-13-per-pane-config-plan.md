# Per-Pane Provider/Model Configuration Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Give each TUI pane independent provider/model configuration via a matrix view UI with column-switching interaction.

**Architecture:** Add `current_provider` and `current_model` fields to the `Pane` struct. Replace the Main Menu's separate Provider/Model entries with a single "Config" entry that opens a matrix view popup. The matrix shows all panes as rows, Provider and Model as columns, with Tab to switch columns, j/k to switch rows, and Enter to edit the cell value. Provider/Model sub-menus now return to the matrix (not Main Menu) and are target-aware. The `update_proxy_config` function sends per-pane config using each pane's own provider+model.

**Tech Stack:** Rust, ratatui (TUI framework), crossterm (terminal backend), legion-db (Provider data), reqwest (HTTP control API)

---

## File Map

All files are under `crates/legion-tui/src/`:

| File | Role | What changes |
|------|------|-------------|
| `app.rs` | App state + enums + menu logic | New enums, Pane fields, matrix navigation, target-aware selection |
| `ui.rs` | Rendering | Matrix view popup, updated header/pane titles, updated main menu |
| `input.rs` | Key routing | Matrix key handling, target-aware Esc/back navigation, per-pane config update |
| `lib.rs` | Entry points | Pane init inherits global defaults (trivial) |

No new files created. No dependency changes.

---

### Task 1: Add new enums and Pane fields to app.rs

This task adds the data structures. No logic changes yet — just the types and fields.

**Files:**
- Modify: `crates/legion-tui/src/app.rs:17-74`

**Step 1: Add `Matrix` variant to `PopupMenu` and new enums**

In `app.rs`, change the `PopupMenu` enum and add two new enums right after it:

```rust
/// Popup menu types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupMenu {
    Main,
    Provider,
    Model,
    Matrix, // NEW
}

/// Which column is active in the matrix view
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixCol {
    Provider,
    Model,
}

/// Target for provider/model assignment
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelTarget {
    Pane(usize),
    AllWorkers,
    AllPanes,
}
```

**Step 2: Add per-pane provider/model fields to `Pane`**

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

**Step 3: Add matrix state fields to `App`**

Add these three fields to the `App` struct, in a new `// Matrix navigation` comment block after the existing `// Menu navigation` block:

```rust
    // Matrix navigation
    pub matrix_row: usize,
    pub matrix_col: MatrixCol,
    pub model_target: Option<ModelTarget>,
```

**Step 4: Initialize new fields in `App::new()`**

Add to the `Self { ... }` block:

```rust
            matrix_row: 0,
            matrix_col: MatrixCol::Provider,
            model_target: None,
```

**Step 5: Set pane fields in `add_pane()`**

Update the `self.panes.push(Pane { ... })` call to include:

```rust
        self.panes.push(Pane {
            pty,
            proxy_port,
            control_port,
            label,
            current_provider: self.current_provider,
            current_model: self.current_model.clone(),
        });
```

**Step 6: Build and verify compilation**

Run: `cargo build -p legion-tui 2>&1 | head -20`
Expected: Compiles with warnings (unused fields) but no errors.

**Step 7: Commit**

```bash
git add crates/legion-tui/src/app.rs
git commit -m "feat(tui): add per-pane provider/model data structures

Add MatrixCol, ModelTarget enums, Matrix popup variant.
Add current_provider and current_model to Pane struct.
Add matrix_row, matrix_col, model_target to App state."
```

---

### Task 2: Add matrix navigation methods to App

This task adds all the App methods needed for the matrix view. No UI or input changes yet.

**Files:**
- Modify: `crates/legion-tui/src/app.rs`

**Step 1: Simplify MainMenuItem enum**

Replace `MainMenuItem` and its impl:

```rust
/// Main menu items
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainMenuItem {
    Config,
    Quit,
}

impl MainMenuItem {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Config => "Config",
            Self::Quit => "Quit",
        }
    }
}
```

Update `main_menu_items()`:

```rust
    pub fn main_menu_items() -> &'static [MainMenuItem] {
        &[MainMenuItem::Config, MainMenuItem::Quit]
    }
```

**Step 2: Update `enter_submenu()` for new menu structure**

Replace the entire `enter_submenu()` method:

```rust
    pub fn enter_submenu(&mut self) {
        if let AppMode::Popup(PopupMenu::Main) = self.mode {
            let items = Self::main_menu_items();
            if self.menu_index < items.len() {
                match items[self.menu_index] {
                    MainMenuItem::Config => {
                        self.mode = AppMode::Popup(PopupMenu::Matrix);
                        self.matrix_row = 0;
                        self.matrix_col = MatrixCol::Provider;
                    }
                    MainMenuItem::Quit => {
                        self.should_quit = true;
                    }
                }
            }
        }
    }
```

**Step 3: Add matrix row count helper**

```rust
    /// Total selectable rows in matrix: panes + "All Workers" (if squad) + "All Panes"
    pub fn matrix_row_count(&self) -> usize {
        let base = self.panes.len();
        if self.is_squad() {
            base + 2 // + All Workers + All Panes
        } else {
            base // single pane: just the one row, no batch options
        }
    }
```

**Step 4: Add `matrix_target()` helper — converts current row to ModelTarget**

```rust
    /// Convert current matrix_row to a ModelTarget
    pub fn matrix_target(&self) -> ModelTarget {
        let pane_count = self.panes.len();
        if self.matrix_row < pane_count {
            ModelTarget::Pane(self.matrix_row)
        } else if self.matrix_row == pane_count {
            ModelTarget::AllWorkers
        } else {
            ModelTarget::AllPanes
        }
    }
```

**Step 5: Add `matrix_enter()` — opens Provider or Model sub-menu from matrix**

```rust
    /// From the matrix, open the provider or model picker for the current cell
    pub fn matrix_enter(&mut self) {
        let target = self.matrix_target();
        self.model_target = Some(target);
        match self.matrix_col {
            MatrixCol::Provider => {
                self.mode = AppMode::Popup(PopupMenu::Provider);
                // Pre-select current provider for the target pane
                self.submenu_index = match target {
                    ModelTarget::Pane(i) => self.panes.get(i)
                        .and_then(|p| p.current_provider)
                        .unwrap_or(0),
                    _ => self.current_provider.unwrap_or(0),
                };
            }
            MatrixCol::Model => {
                self.mode = AppMode::Popup(PopupMenu::Model);
                // Pre-select current model for the target pane
                self.submenu_index = match target {
                    ModelTarget::Pane(i) => {
                        let pane_model = self.panes.get(i).and_then(|p| p.current_model.as_ref());
                        let pane_provider = self.panes.get(i).and_then(|p| p.current_provider);
                        pane_provider
                            .and_then(|pi| self.providers.get(pi))
                            .and_then(|p| p.models.as_ref())
                            .and_then(|models| {
                                pane_model.and_then(|m| models.iter().position(|x| x == m))
                            })
                            .unwrap_or(0)
                    }
                    _ => self.get_current_model_index().unwrap_or(0),
                };
            }
        }
    }
```

**Step 6: Add `back_to_matrix()` — return from sub-menu to matrix**

```rust
    /// Return from Provider/Model sub-menu back to the matrix view
    pub fn back_to_matrix(&mut self) {
        self.model_target = None;
        self.mode = AppMode::Popup(PopupMenu::Matrix);
    }
```

**Step 7: Rewrite `select_submenu_item()` to be target-aware**

Replace the entire method:

```rust
    pub fn select_submenu_item(&mut self) {
        match self.mode {
            AppMode::Popup(PopupMenu::Provider) => {
                if self.submenu_index < self.providers.len() {
                    let first_model = self.providers.get(self.submenu_index)
                        .and_then(|p| p.models.as_ref())
                        .and_then(|m| m.first().cloned());

                    match self.model_target {
                        Some(ModelTarget::Pane(i)) => {
                            if let Some(pane) = self.panes.get_mut(i) {
                                pane.current_provider = Some(self.submenu_index);
                                pane.current_model = first_model;
                            }
                        }
                        Some(ModelTarget::AllWorkers) => {
                            for pane in self.panes.iter_mut().skip(1) {
                                pane.current_provider = Some(self.submenu_index);
                                pane.current_model = first_model.clone();
                            }
                        }
                        Some(ModelTarget::AllPanes) | None => {
                            self.current_provider = Some(self.submenu_index);
                            self.current_model = first_model.clone();
                            for pane in self.panes.iter_mut() {
                                pane.current_provider = Some(self.submenu_index);
                                pane.current_model = first_model.clone();
                            }
                        }
                    }
                    self.provider_connected = true;
                }
                // Return to matrix if we came from there, else main
                if self.model_target.is_some() {
                    self.back_to_matrix();
                } else {
                    self.mode = AppMode::Popup(PopupMenu::Main);
                }
            }
            AppMode::Popup(PopupMenu::Model) => {
                let model_name = self.target_provider_models()
                    .and_then(|models| models.get(self.submenu_index).cloned());

                if let Some(model) = model_name {
                    match self.model_target {
                        Some(ModelTarget::Pane(i)) => {
                            if let Some(pane) = self.panes.get_mut(i) {
                                pane.current_model = Some(model);
                            }
                        }
                        Some(ModelTarget::AllWorkers) => {
                            for pane in self.panes.iter_mut().skip(1) {
                                pane.current_model = Some(model.clone());
                            }
                        }
                        Some(ModelTarget::AllPanes) | None => {
                            self.current_model = Some(model.clone());
                            for pane in self.panes.iter_mut() {
                                pane.current_model = Some(model.clone());
                            }
                        }
                    }
                }
                if self.model_target.is_some() {
                    self.back_to_matrix();
                } else {
                    self.mode = AppMode::Popup(PopupMenu::Main);
                }
            }
            _ => {}
        }
    }
```

**Step 8: Add `target_provider_models()` helper**

This returns the models list for the current target's provider (used by model selection and UI):

```rust
    /// Get models for the current target's provider
    pub fn target_provider_models(&self) -> Option<&Vec<String>> {
        let provider_idx = match self.model_target {
            Some(ModelTarget::Pane(i)) => self.panes.get(i).and_then(|p| p.current_provider),
            _ => self.current_provider,
        };
        provider_idx
            .and_then(|i| self.providers.get(i))
            .and_then(|p| p.models.as_ref())
    }

    /// Get the target label for sub-menu titles (e.g. "Leader", "All Workers")
    pub fn target_label(&self) -> &str {
        match self.model_target {
            Some(ModelTarget::Pane(i)) => self.panes.get(i)
                .map(|p| p.label.as_str())
                .unwrap_or("Pane"),
            Some(ModelTarget::AllWorkers) => "All Workers",
            Some(ModelTarget::AllPanes) => "All Panes",
            None => "All",
        }
    }
```

**Step 9: Update `menu_up()` and `menu_down()` to handle Matrix**

Add a `PopupMenu::Matrix` arm to the `len` calculation in both methods:

In `menu_up()` and `menu_down()`, in the `let len = match self.mode { ... }` block, add:

```rust
            AppMode::Popup(PopupMenu::Matrix) => self.matrix_row_count(),
```

And update the `let idx = match self.mode { ... }` block to handle Matrix:

```rust
        let idx = match self.mode {
            AppMode::Popup(PopupMenu::Main) => &mut self.menu_index,
            AppMode::Popup(PopupMenu::Matrix) => &mut self.matrix_row,
            _ => &mut self.submenu_index,
        };
```

**Step 10: Add `matrix_col_toggle()` — Tab toggles column**

```rust
    /// Toggle the active column in the matrix view
    pub fn matrix_col_toggle(&mut self) {
        self.matrix_col = match self.matrix_col {
            MatrixCol::Provider => MatrixCol::Model,
            MatrixCol::Model => MatrixCol::Provider,
        };
    }
```

**Step 11: Update `back_to_main_menu()` for clarity**

No change needed — the existing `back_to_main_menu()` already sets mode to `PopupMenu::Main`, which is correct for Esc from the Matrix view.

**Step 12: Build and verify compilation**

Run: `cargo build -p legion-tui 2>&1 | head -30`
Expected: Compiles (some warnings about unused imports in ui.rs are expected — we'll fix those in Task 3).

**Step 13: Commit**

```bash
git add crates/legion-tui/src/app.rs
git commit -m "feat(tui): add matrix navigation logic and target-aware selection

Simplify MainMenuItem to Config+Quit. Add matrix_enter(), back_to_matrix(),
matrix_target(), target_provider_models(), target_label(). Rewrite
select_submenu_item() to apply changes per-pane based on ModelTarget."
```

---

### Task 3: Update UI rendering for matrix view

**Files:**
- Modify: `crates/legion-tui/src/ui.rs`

**Step 1: Update imports**

Replace the import line:

```rust
use crate::app::{App, AppMode, MainMenuItem, PopupMenu};
```

with:

```rust
use crate::app::{App, AppMode, MainMenuItem, MatrixCol, PopupMenu};
```

**Step 2: Update `draw_popup()` to handle Matrix**

Replace the entire `draw_popup()` function:

```rust
fn draw_popup(frame: &mut Frame, app: &App, menu: PopupMenu) {
    let area = centered_rect(60, 60, frame.area());
    frame.render_widget(Clear, area);

    match menu {
        PopupMenu::Main => draw_main_menu(frame, app, area),
        PopupMenu::Matrix => draw_matrix(frame, app, area),
        PopupMenu::Provider => draw_provider_menu(frame, app, area),
        PopupMenu::Model => draw_model_menu(frame, app, area),
    }
}
```

**Step 3: Rewrite `draw_main_menu()` for Config + Quit**

Replace the entire function:

```rust
fn draw_main_menu(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Legion [ESC] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::DarkGray));

    let items: Vec<ListItem> = App::main_menu_items()
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let selected = i == app.menu_index;
            let prefix = if selected { "> " } else { "  " };
            let value = match item {
                MainMenuItem::Config => {
                    let n = app.panes.len();
                    if n == 0 {
                        "[no panes]".to_string()
                    } else {
                        format!("[{} pane{}]", n, if n == 1 { "" } else { "s" })
                    }
                }
                MainMenuItem::Quit => String::new(),
            };

            let style = if selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            if value.is_empty() {
                ListItem::new(Line::from(vec![
                    Span::raw(prefix),
                    Span::styled(item.label(), style),
                ]))
            } else {
                let pad = " ".repeat(20usize.saturating_sub(prefix.len() + item.label().len()));
                ListItem::new(Line::from(vec![
                    Span::raw(prefix),
                    Span::styled(item.label(), style),
                    Span::raw(pad),
                    Span::styled(value, Style::default().fg(Color::DarkGray)),
                ]))
            }
        })
        .collect();

    // Separator before Quit
    let mut final_items = Vec::new();
    for (i, item) in items.into_iter().enumerate() {
        if i == 1 {
            final_items.push(ListItem::new(Line::from(Span::styled(
                "  \u{2500}".repeat(12),
                Style::default().fg(Color::DarkGray),
            ))));
        }
        final_items.push(item);
    }

    frame.render_widget(List::new(final_items).block(block), area);
}
```

**Step 4: Add `draw_matrix()` — the matrix view**

Add this new function:

```rust
/// Matrix view: rows = panes + batch targets, columns = Provider | Model
fn draw_matrix(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Configuration [ESC] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::DarkGray));

    let pane_count = app.panes.len();
    let mut items: Vec<ListItem> = Vec::new();

    // Column header
    items.push(ListItem::new(Line::from(vec![
        Span::styled("  Pane            ", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
        Span::styled("Provider          ", Style::default().fg(
            if app.matrix_col == MatrixCol::Provider { Color::Yellow } else { Color::DarkGray }
        ).add_modifier(Modifier::BOLD)),
        Span::styled("Model", Style::default().fg(
            if app.matrix_col == MatrixCol::Model { Color::Magenta } else { Color::DarkGray }
        ).add_modifier(Modifier::BOLD)),
    ])));

    // Pane rows
    for (i, pane) in app.panes.iter().enumerate() {
        let is_row = app.matrix_row == i;
        items.push(matrix_row_item(app, is_row, &pane.label, pane.current_provider, pane.current_model.as_deref()));
    }

    // Separator + batch rows (squad mode only)
    if app.is_squad() {
        items.push(ListItem::new(Line::from(Span::styled(
            "  \u{2500}".repeat(20),
            Style::default().fg(Color::DarkGray),
        ))));

        let aw_selected = app.matrix_row == pane_count;
        items.push(matrix_row_item(app, aw_selected, "All Workers", None, None));

        let ap_selected = app.matrix_row == pane_count + 1;
        items.push(matrix_row_item(app, ap_selected, "All Panes", None, None));
    }

    // Footer hint inside the popup
    items.push(ListItem::new(Line::from(Span::raw(""))));
    items.push(ListItem::new(Line::from(vec![
        Span::styled("  Tab", Style::default().fg(Color::Yellow)),
        Span::styled(": Column  ", Style::default().fg(Color::DarkGray)),
        Span::styled("Enter", Style::default().fg(Color::Yellow)),
        Span::styled(": Edit  ", Style::default().fg(Color::DarkGray)),
        Span::styled("Esc", Style::default().fg(Color::Yellow)),
        Span::styled(": Back", Style::default().fg(Color::DarkGray)),
    ])));

    frame.render_widget(List::new(items).block(block), area);
}

/// Build one row of the matrix view
fn matrix_row_item<'a>(
    app: &App,
    is_selected_row: bool,
    label: &str,
    provider_idx: Option<usize>,
    model: Option<&str>,
) -> ListItem<'a> {
    let prefix = if is_selected_row { "> " } else { "  " };

    let provider_name = provider_idx
        .and_then(|i| app.providers.get(i))
        .map(|p| p.name.as_str())
        .unwrap_or("--");
    let model_name = model.unwrap_or("--");

    // Highlight logic: row selected + column active = brightest
    let provider_style = if is_selected_row && app.matrix_col == MatrixCol::Provider {
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else if is_selected_row {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let model_style = if is_selected_row && app.matrix_col == MatrixCol::Model {
        Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)
    } else if is_selected_row {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let row_style = if is_selected_row {
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    // Pad columns for alignment
    let label_padded = format!("{:<16}", label);
    let provider_padded = format!("{:<18}", format!("[{}]", provider_name));

    ListItem::new(Line::from(vec![
        Span::raw(prefix.to_string()),
        Span::styled(label_padded, row_style),
        Span::styled(provider_padded, provider_style),
        Span::styled(format!("[{}]", model_name), model_style),
    ]))
}
```

**Step 5: Update `draw_provider_menu()` title to show target**

Replace the title line:

```rust
        .title(format!(" Select Provider for {} [ESC] ", app.target_label()))
```

**Step 6: Update `draw_model_menu()` for target-aware model list**

Replace the title line:

```rust
        .title(format!(" Select Model for {} [ESC] ", app.target_label()))
```

Replace the model source line (`if let Some(models) = app.get_current_provider_models()`):

```rust
    if let Some(models) = app.target_provider_models() {
```

And update the `current` check inside the map closure — replace:

```rust
                let current = app.current_model.as_deref() == Some(model);
```

with:

```rust
                let current = match app.model_target {
                    Some(crate::app::ModelTarget::Pane(pi)) => app.panes.get(pi)
                        .and_then(|p| p.current_model.as_deref()) == Some(model),
                    _ => app.current_model.as_deref() == Some(model),
                };
```

**Step 7: Update `draw_provider_menu()` current-provider check**

In `draw_provider_menu()`, replace:

```rust
            let current = app.current_provider == Some(i);
```

with:

```rust
            let current = match app.model_target {
                Some(crate::app::ModelTarget::Pane(pi)) => app.panes.get(pi)
                    .and_then(|p| p.current_provider) == Some(i),
                _ => app.current_provider == Some(i),
            };
```

**Step 8: Update header to show focused pane's provider/model**

In `draw_header()`, replace:

```rust
    let provider_name = app
        .get_current_provider()
        .map(|p| p.name.as_str())
        .unwrap_or("No Provider");
    let model_name = app.current_model.as_deref().unwrap_or("No Model");
```

with:

```rust
    let focused_pane = app.panes.get(app.focused_pane);
    let provider_name = focused_pane
        .and_then(|p| p.current_provider)
        .and_then(|i| app.providers.get(i))
        .map(|p| p.name.as_str())
        .unwrap_or("No Provider");
    let model_name = focused_pane
        .and_then(|p| p.current_model.as_deref())
        .unwrap_or("No Model");
```

**Step 9: Update pane title to show model in squad mode**

In `draw_pane()`, replace:

```rust
    let title = if app.is_squad() {
        format!(" {} (:{}) ", pane.label, pane.proxy_port)
    } else {
        " Claude Code ".to_string()
    };
```

with:

```rust
    let title = if app.is_squad() {
        let model = pane.current_model.as_deref().unwrap_or("--");
        format!(" {} | {} ", pane.label, model)
    } else {
        " Claude Code ".to_string()
    };
```

**Step 10: Update footer for Matrix mode**

In `draw_footer()`, update the `AppMode::Popup(_)` arm to show matrix-specific hints when in Matrix mode:

Replace the entire `AppMode::Popup(_)` block:

```rust
            AppMode::Popup(popup) => match popup {
                PopupMenu::Matrix => vec![
                    Span::styled(" Tab", Style::default().fg(Color::Yellow)),
                    Span::styled(": Column ", Style::default().fg(Color::DarkGray)),
                    Span::styled("j/k", Style::default().fg(Color::Yellow)),
                    Span::styled(": Row ", Style::default().fg(Color::DarkGray)),
                    Span::styled("Enter", Style::default().fg(Color::Yellow)),
                    Span::styled(": Edit ", Style::default().fg(Color::DarkGray)),
                    Span::styled("Esc", Style::default().fg(Color::Yellow)),
                    Span::styled(": Back", Style::default().fg(Color::DarkGray)),
                ],
                _ => vec![
                    Span::styled(" j/k", Style::default().fg(Color::Yellow)),
                    Span::styled(": Navigate ", Style::default().fg(Color::DarkGray)),
                    Span::styled("Enter", Style::default().fg(Color::Yellow)),
                    Span::styled(": Select ", Style::default().fg(Color::DarkGray)),
                    Span::styled("Esc", Style::default().fg(Color::Yellow)),
                    Span::styled(": Close", Style::default().fg(Color::DarkGray)),
                ],
            },
```

Note: this replaces what was previously a single `AppMode::Popup(_) => vec![...]` arm.

**Step 11: Build and verify**

Run: `cargo build -p legion-tui 2>&1 | head -30`
Expected: Compiles successfully.

**Step 12: Commit**

```bash
git add crates/legion-tui/src/ui.rs
git commit -m "feat(tui): add matrix view UI for per-pane config

New draw_matrix() renders pane rows with Provider/Model columns.
Header and pane titles now show focused pane's provider/model.
Main menu simplified to Config + Quit."
```

---

### Task 4: Update input handling for matrix navigation

**Files:**
- Modify: `crates/legion-tui/src/input.rs`

**Step 1: Update imports**

Replace:

```rust
use crate::app::{App, AppMode, PopupMenu};
```

with:

```rust
use crate::app::{App, AppMode, ModelTarget, PopupMenu};
```

**Step 2: Rewrite `handle_popup_mode()` with matrix and target-aware navigation**

Replace the entire `handle_popup_mode()` function:

```rust
fn handle_popup_mode(app: &mut App, key: KeyEvent) -> InputResult {
    // Ctrl+P also closes popup
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('p') {
        app.toggle_popup();
        return InputResult::Continue;
    }

    match app.mode {
        AppMode::Popup(PopupMenu::Matrix) => handle_matrix_keys(app, key),
        AppMode::Popup(PopupMenu::Main) => handle_main_menu_keys(app, key),
        AppMode::Popup(PopupMenu::Provider) | AppMode::Popup(PopupMenu::Model) => {
            handle_submenu_keys(app, key)
        }
        _ => {}
    }

    InputResult::Continue
}

fn handle_matrix_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => app.back_to_main_menu(),
        KeyCode::Up | KeyCode::Char('k') => app.menu_up(),
        KeyCode::Down | KeyCode::Char('j') => app.menu_down(),
        KeyCode::Tab | KeyCode::Left | KeyCode::Right
        | KeyCode::Char('h') | KeyCode::Char('l') => app.matrix_col_toggle(),
        KeyCode::Enter => {
            app.matrix_enter();
        }
        _ => {}
    }
}

fn handle_main_menu_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => app.toggle_popup(),
        KeyCode::Up | KeyCode::Char('k') => app.menu_up(),
        KeyCode::Down | KeyCode::Char('j') => app.menu_down(),
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => app.enter_submenu(),
        _ => {}
    }
}

fn handle_submenu_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') => {
            // Go back to matrix if we came from there, else main menu
            if app.model_target.is_some() {
                app.back_to_matrix();
            } else {
                app.back_to_main_menu();
            }
        }
        KeyCode::Up | KeyCode::Char('k') => app.menu_up(),
        KeyCode::Down | KeyCode::Char('j') => app.menu_down(),
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            app.select_submenu_item();
            update_proxy_config(app);
        }
        _ => {}
    }
}
```

**Step 3: Rewrite `update_proxy_config()` for per-pane config**

Replace the entire function:

```rust
/// After provider/model selection, POST config to affected panes' control APIs
fn update_proxy_config(app: &App) {
    // Build per-pane config tuples: (control_port, base_url, api_format, api_key, model)
    let configs: Vec<(u16, String, String, Option<String>, Option<String>)> = app
        .panes
        .iter()
        .filter_map(|pane| {
            let provider = pane
                .current_provider
                .and_then(|i| app.providers.get(i))?;
            Some((
                pane.control_port,
                provider.base_url.clone(),
                provider.api_format.clone(),
                provider.api_key.clone(),
                pane.current_model.clone(),
            ))
        })
        .collect();

    if configs.is_empty() {
        return;
    }

    tokio::spawn(async move {
        let client = reqwest::Client::new();
        for (port, base_url, api_format, api_key, model) in configs {
            let body = serde_json::json!({
                "target_url": base_url,
                "api_format": api_format,
                "api_key": api_key,
                "model": model,
            });
            let _ = client
                .post(format!("http://127.0.0.1:{}/legion/config", port))
                .json(&body)
                .send()
                .await;
        }
    });
}
```

**Step 4: Build and verify**

Run: `cargo build -p legion-tui 2>&1 | head -30`
Expected: Compiles successfully.

**Step 5: Commit**

```bash
git add crates/legion-tui/src/input.rs
git commit -m "feat(tui): matrix input handling and per-pane config updates

Split popup key handling into matrix/main/submenu handlers.
Esc/back from Provider/Model returns to Matrix when target is set.
update_proxy_config() now sends per-pane provider+model configs."
```

---

### Task 5: Build full project and verify

**Files:**
- No changes — verification only

**Step 1: Full build**

Run: `cargo build 2>&1 | tail -20`
Expected: All crates compile successfully.

**Step 2: Check for warnings**

Run: `cargo build -p legion-tui 2>&1 | grep warning`
Expected: No warnings (or only pre-existing ones like `unused variable: control_port` in `run_popup`).

**Step 3: Fix any unused import warnings**

If there are warnings about unused `ModelTarget` in `input.rs`, verify the import is actually used in the `handle_submenu_keys` function. If `ModelTarget` is not directly referenced in input.rs (it's only used via `app.model_target.is_some()`), remove the import:

```rust
use crate::app::{App, AppMode, PopupMenu};
```

**Step 4: Final build check**

Run: `cargo build 2>&1 | tail -5`
Expected: Clean build.

**Step 5: Commit any fixes**

```bash
git add -A
git commit -m "fix: clean up unused imports after per-pane config refactor"
```

---

## Summary of Changes

| File | Lines changed (approx) | What |
|------|----------------------|------|
| `app.rs` | ~120 lines modified | New enums (MatrixCol, ModelTarget, Matrix popup), Pane fields, matrix navigation methods, target-aware selection |
| `ui.rs` | ~100 lines modified | draw_matrix(), matrix_row_item(), updated header/pane titles, simplified main menu, updated provider/model menus for target |
| `input.rs` | ~60 lines modified | Split handle_popup_mode into matrix/main/submenu handlers, per-pane update_proxy_config |
| `lib.rs` | 0 lines | No changes needed (add_pane already inherits from App defaults) |

## State Machine (final)

```
Normal
  Ctrl+P -> Popup(Main)

Popup(Main)
  Esc -> Normal
  Enter on Config -> Popup(Matrix)
  Enter on Quit -> exit

Popup(Matrix)
  j/k -> navigate rows
  Tab/Left/Right -> toggle column
  Enter -> Popup(Provider) or Popup(Model) with model_target set
  Esc -> Popup(Main)

Popup(Provider) [with model_target]
  j/k -> navigate
  Enter -> apply to target, back to Popup(Matrix)
  Esc -> Popup(Matrix)

Popup(Model) [with model_target]
  j/k -> navigate
  Enter -> apply to target, back to Popup(Matrix)
  Esc -> Popup(Matrix)
```
