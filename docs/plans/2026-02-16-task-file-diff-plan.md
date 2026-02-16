# Task File Change Diff Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 为每个 Working/Done/Error ticket 提供文件变更查看功能，左右分栏显示文件列表和 diff 内容。

**Architecture:** Done/Error 时缓存 diff 到 DB，Working 时实时获取。通过 git diff 对比 worker branch vs session Leader ref。独立全屏 popup 展示。

**Tech Stack:** Rust, ratatui, git CLI, SQLite

---

### Task 1: DB Schema — ticket_diffs 表

**Files:**
- Modify: `crates/legion-db/src/schema.rs`
- Modify: `crates/legion-db/src/repo.rs`

**改动:**

1. `schema.rs` — 在 `CREATE_TABLES` 末尾添加:
```sql
CREATE TABLE IF NOT EXISTS ticket_diffs (
    ticket_id INTEGER PRIMARY KEY,
    session_name TEXT NOT NULL,
    diff_content TEXT NOT NULL,
    file_summary TEXT NOT NULL,
    cached_at INTEGER NOT NULL
);
```

2. `repo.rs` — 新增 struct 和方法:

```rust
/// 单个文件的 diff 摘要
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileDiffSummary {
    pub path: String,
    pub status: String,      // "A", "M", "D"
    pub additions: usize,
    pub deletions: usize,
}

/// 缓存的 ticket diff
pub struct TicketDiffRow {
    pub ticket_id: i64,
    pub session_name: String,
    pub diff_content: String,
    pub file_summary: Vec<FileDiffSummary>,  // 从 JSON 反序列化
    pub cached_at: i64,
}
```

方法:
- `save_ticket_diff(ticket_id, session_name, diff_content, file_summary_json, cached_at)` — INSERT OR REPLACE
- `get_ticket_diff(ticket_id) -> Option<TicketDiffRow>` — SELECT, 解析 file_summary JSON
- `delete_ticket_diff(ticket_id)` — DELETE（ticket 删除时清理）

3. 修改现有 `delete_ticket()` — 同时删除 ticket_diffs 中的记录

**Commit:** `feat(db): add ticket_diffs table for file change caching`

---

### Task 2: Git Diff 获取模块

**Files:**
- Create: `crates/legion-tui/src/diff.rs`
- Modify: `crates/legion-tui/src/lib.rs` (添加 `pub mod diff;`)

**改动:**

新建 `diff.rs`，包含:

```rust
use std::path::Path;
use std::process::Command;
use anyhow::Result;

/// 解析后的 diff 数据
#[derive(Debug, Clone)]
pub struct DiffData {
    pub files: Vec<DiffFile>,
    pub raw_diff: String,
}

#[derive(Debug, Clone)]
pub struct DiffFile {
    pub path: String,
    pub status: String,       // "A", "M", "D"
    pub additions: usize,
    pub deletions: usize,
    pub hunks: Vec<String>,   // 该文件的 diff 行
}

/// 获取 worker worktree 相对于 leader ref 的 diff
pub fn get_worktree_diff(worktree_path: &Path, leader_ref: &str) -> Result<DiffData> {
    // 1. git merge-base <leader_ref> HEAD
    // 2. git diff <merge_base>..HEAD --unified=3
    // 3. Working 状态追加: git diff + git diff --cached
    // 4. 解析 diff 输出为 DiffData
}

/// 获取 leader ref 名称
pub fn get_leader_ref(project_path: &Path, session_name: &str, is_default: bool) -> String {
    // Default session: 用 worktree::default_branch()
    // 其他 session: "legion/<session>/leader"
}

/// 解析 git diff 输出为结构化数据
fn parse_diff(raw: &str) -> Vec<DiffFile> {
    // 按 "diff --git a/... b/..." 分割
    // 统计每个文件的 +/- 行数
    // 识别文件状态: new file (A), deleted file (D), 其他 (M)
}

/// 获取 numstat 用于快速统计
fn get_diff_numstat(worktree_path: &Path, base: &str) -> Result<Vec<(String, usize, usize)>> {
    // git diff --numstat <base>..HEAD
    // 解析: additions \t deletions \t path
}
```

**Commit:** `feat(tui): add diff module for git diff retrieval and parsing`

---

### Task 3: App State — Diff Popup 状态

**Files:**
- Modify: `crates/legion-tui/src/app.rs`

**改动:**

1. `PopupMenu` enum 添加:
```rust
FileDiff,
```

2. `App` struct 添加 diff popup 状态字段:
```rust
// File diff popup state
pub diff_ticket_id: usize,
pub diff_data: Option<DiffData>,
pub diff_file_selected: usize,     // 左栏选中文件 index
pub diff_scroll: usize,            // 右栏 diff 内容滚动偏移
pub diff_loading: bool,            // 正在加载 diff
pub diff_error: Option<String>,    // diff 获取失败的错误信息
```

3. 初始化这些字段（在 `App::new()` 或对应位置）全部默认为 0/None/false。

**Commit:** `feat(app): add FileDiff popup state`

---

### Task 4: Diff 缓存集成到 Engine

**Files:**
- Modify: `crates/legion-core/src/orchestrate/engine.rs`

**改动:**

在 `report_iteration()` 中，当 ticket 变为 Done 或 Error 时，在 `persist_ticket_update()` 之后异步缓存 diff:

```rust
if success || ticket.iteration >= ticket.max_iterations {
    // ... existing Done/Error logic ...

    // Cache diff asynchronously
    if let Some(worker_id) = snap.assigned_worker {
        let db = self.db.clone();
        let session = snap.session_name.clone();
        let project = self.project_path.clone();
        let is_default = self.is_default_session;
        let ticket_id = snap.id;
        tokio::spawn(async move {
            if let Err(e) = cache_ticket_diff(db, project, session, is_default, worker_id, ticket_id).await {
                tracing::warn!("Failed to cache diff for ticket {}: {}", ticket_id, e);
            }
        });
    }
}
```

新增辅助函数 `cache_ticket_diff()`:
- 构造 worktree path
- 调用 `diff::get_worktree_diff()`
- 构造 file_summary JSON
- 调用 `db.save_ticket_diff()`

需要确认 engine 是否有 `project_path` 和 `is_default_session` 信息，可能需要在 `OrchestrationEngine` 中添加这些字段。

**Commit:** `feat(engine): cache diff on ticket Done/Error`

---

### Task 5: Input — [f] 快捷键和 Diff Popup 交互

**Files:**
- Modify: `crates/legion-tui/src/input.rs`

**改动:**

1. Ticket board 按键处理（约 line 56-124 区域）中添加 `f` 键:
```rust
KeyCode::Char('f') => {
    // 获取当前选中的 ticket
    // 只允许 Working/Done/Error（不允许 Queued）
    // 设置 diff_ticket_id, diff_loading = true
    // 切换到 Popup(FileDiff)
    // 异步获取 diff 数据（Done/Error 从 DB 读缓存，Working 实时获取）
}
```

2. 新增 `handle_file_diff_keys(app, key)`:
```rust
match key.code {
    KeyCode::Up => {
        // 文件列表上移
        if app.diff_file_selected > 0 {
            app.diff_file_selected -= 1;
            app.diff_scroll = 0;  // 切换文件时重置滚动
        }
    }
    KeyCode::Down => {
        // 文件列表下移
        if let Some(ref data) = app.diff_data {
            if app.diff_file_selected < data.files.len().saturating_sub(1) {
                app.diff_file_selected += 1;
                app.diff_scroll = 0;
            }
        }
    }
    KeyCode::Char('j') => {
        // diff 内容向下滚动一行
        app.diff_scroll = app.diff_scroll.saturating_add(1);
    }
    KeyCode::Char('k') => {
        // diff 内容向上滚动一行
        app.diff_scroll = app.diff_scroll.saturating_sub(1);
    }
    KeyCode::PageDown => {
        // diff 内容向下翻页（以可视区域高度为单位）
        app.diff_scroll = app.diff_scroll.saturating_add(20);
    }
    KeyCode::PageUp => {
        app.diff_scroll = app.diff_scroll.saturating_sub(20);
    }
    KeyCode::Home => { app.diff_scroll = 0; }
    KeyCode::End => {
        // 跳到最后
        // 需要计算当前文件 diff 总行数
    }
    KeyCode::Esc => {
        // 关闭 diff popup
        app.mode = AppMode::Normal;
        app.diff_data = None;
    }
}
```

3. 鼠标滚轮处理 — 在已有的 mouse event handler 中添加:
```rust
// 在 FileDiff popup 模式下
// 检测鼠标位置在左栏还是右栏
// 左栏滚动 → 切换文件选择
// 右栏滚动 → 滚动 diff 内容
```

4. 在 main input dispatcher 中添加 `PopupMenu::FileDiff => handle_file_diff_keys(app, key)`。

**Commit:** `feat(input): add [f] key and diff popup navigation`

---

### Task 6: UI — Diff Popup 渲染

**Files:**
- Modify: `crates/legion-tui/src/ui.rs`

**改动:**

1. `draw_popup()` match 中添加:
```rust
PopupMenu::FileDiff => draw_file_diff(frame, app, popup_area),
```

2. Popup 尺寸: `PopupMenu::FileDiff => (95, 90)` (接近全屏)

3. 新增 `draw_file_diff(frame, app, area)`:

```rust
fn draw_file_diff(frame: &mut Frame, app: &App, area: Rect) {
    // 标题栏: "#<id> <title> — [<status>]"

    if app.diff_loading {
        // 显示 "Loading diff..." 居中
        return;
    }

    if let Some(ref error) = app.diff_error {
        // 显示错误信息
        return;
    }

    let Some(ref data) = app.diff_data else { return; };

    if data.files.is_empty() {
        // 显示 "No file changes"
        return;
    }

    // 水平分割: 左 35% | 右 65%
    let chunks = Layout::horizontal([
        Constraint::Percentage(35),
        Constraint::Percentage(65),
    ]).split(inner_area);

    // === 左栏: 文件列表 ===
    // 标题: "Files (N changed, +X -Y)"
    // 每行: "> path/to/file.rs   +45 -2"
    //   - 选中行高亮背景
    //   - A: 绿色, M: 黄色, D: 红色
    //   - 右对齐 "+N -M" 统计
    // 滚动条 (如果文件数超过可视区域)

    // === 右栏: Diff 内容 ===
    // 标题: 当前选中文件名
    // 内容: 该文件的 diff hunks
    //   - "+" 行: 绿色前景
    //   - "-" 行: 红色前景
    //   - "@@" 行: 青色前景
    //   - 其他: 默认灰色
    // 滚动条
    // 注意 diff_scroll 需要 clamp 到实际行数范围

    // === Footer ===
    // "↑↓ Files │ j/k Scroll │ PgUp/PgDn Page │ Esc Close"
}
```

4. Action bar 更新 — 在 `draw_task_board()` 的 action bar 区域:
- Working/Done/Error ticket: 添加 `[f] Files`
- Queued ticket: 不显示

**Commit:** `feat(ui): add diff popup with split-pane file list and diff content`

---

### Task 7: 异步 Diff 加载

**Files:**
- Modify: `crates/legion-tui/src/input.rs`
- Possibly: `crates/legion-tui/src/app.rs`

**改动:**

`f` 键处理中需要异步加载 diff 数据:

```rust
KeyCode::Char('f') => {
    if let Some(tickets) = &app.ticket_snapshot {
        if let Some(ticket) = tickets.iter().find(|t| t.id == app.board_selected) {
            let status = &ticket.status;
            if matches!(status, TicketStatus::Working | TicketStatus::Done | TicketStatus::Error) {
                app.diff_ticket_id = ticket.id;
                app.diff_file_selected = 0;
                app.diff_scroll = 0;
                app.diff_loading = true;
                app.diff_error = None;
                app.diff_data = None;
                app.mode = AppMode::Popup(PopupMenu::FileDiff);

                // Done/Error: 先查 DB 缓存
                if matches!(status, TicketStatus::Done | TicketStatus::Error) {
                    // 尝试从 DB 读取缓存
                    if let Some(cached) = db.get_ticket_diff(ticket.id as i64) {
                        app.diff_data = Some(parse_cached_diff(cached));
                        app.diff_loading = false;
                        return;
                    }
                }

                // Working 或缓存未命中: 实时获取
                // 需要知道 worker_id → worktree_path
                // 异步 spawn，完成后通过 channel 或直接设置 app state
                let worker_id = ticket.assigned_worker;
                // ... spawn blocking task to get diff ...
            }
        }
    }
}
```

对于异步加载，有两种方式:
- **方式 A**: 使用 `tokio::task::block_in_place` 同步等待（简单，但会短暂阻塞 UI）
- **方式 B**: 用 channel 异步返回（更好的 UX，不阻塞）

推荐方式 A（和现有 retry 逻辑一致），因为 git diff 通常很快（< 100ms）。

**Commit:** `feat(input): async diff loading with DB cache fallback`

---

### Task 8: 清理和集成

**Files:**
- Modify: `crates/legion-db/src/repo.rs` — 确保 `delete_ticket` 和 `clear_completed` 也清理 ticket_diffs
- Modify: `crates/legion-tui/src/input.rs` — 确保删除 ticket 时清理 diff 缓存
- Modify: `crates/legion-core/src/orchestrate/engine.rs` — 确保 clear_completed 清理 diff

**改动:**
- `delete_ticket()` in repo.rs: `DELETE FROM ticket_diffs WHERE ticket_id = ?`
- `clear_completed()` in engine.rs: 对已清理的 ticket_id 调用 `delete_ticket_diff()`
- `delete_session_tickets()` in repo.rs: `DELETE FROM ticket_diffs WHERE session_name = ?`

**Commit:** `feat(db): cleanup ticket_diffs on ticket deletion`

---

### Task 9: Build + 验证

- `cargo build` 通过
- `cargo test` 通过
- 手动验证:
  - 启动 squad 模式，提交 ticket
  - ticket Working 时按 [f] → 显示实时 diff（可能为空如果还没改文件）
  - ticket Done 后按 [f] → 显示缓存的 diff
  - ticket Error 后按 [f] → 显示缓存的 diff
  - 上下键切换文件，j/k 滚动 diff 内容
  - PgUp/PgDn 翻页，Home/End 跳转
  - 鼠标滚轮在两栏分别滚动
  - 滚动条正确显示
  - Esc 关闭 popup
  - Queued ticket 不显示 [f]

**Commit:** 无（验证步骤）
