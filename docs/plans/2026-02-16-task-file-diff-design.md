# Task File Change Diff Design

## Goal

为每个 ticket (Working/Done/Error) 提供文件变更查看功能，通过独立的 diff popup 展示 worker 相对于 session Leader 的代码变更。

## Architecture

### 数据获取

- 每个 worker 在独立 git worktree 的 branch (`legion/<session>/<worker>`) 上工作
- Diff 对比基准：worker branch vs session Leader ref
  - Default session Leader ref = git default branch (main/master)
  - 其他 session Leader ref = `legion/<session>/leader` branch
- 命令：`git diff $(git merge-base <leader-ref> HEAD) HEAD`（在 worker worktree 目录执行）
- Working 状态追加未提交变更：`git diff` + `git diff --cached`

### 缓存策略

- **Done/Error**: 状态转换时自动缓存 diff 到 DB `ticket_diffs` 表
- **Working**: 按 [f] 时实时运行 git diff，不缓存
- **Queued**: 无 worker 分配，不可查看

### DB Schema

```sql
CREATE TABLE IF NOT EXISTS ticket_diffs (
    ticket_id INTEGER PRIMARY KEY,
    session_name TEXT NOT NULL,
    diff_content TEXT NOT NULL,
    file_summary TEXT NOT NULL,  -- JSON: [{path, status, additions, deletions}]
    cached_at INTEGER NOT NULL
);
```

## UI Design

### Diff Popup (全屏弹窗)

```
┌─ #3 Implement auth endpoint ── [Done] ─────────────────────┐
│ Files (5 changed, +120 -30)    │ src/auth/handler.rs       │
│                                │                           │
│ > src/auth/handler.rs   +45 -2 │ @@ -10,6 +10,51 @@       │
│   src/auth/mod.rs       +3  -0 │  use axum::Router;        │
│   src/db/users.rs       +32 -8 │ +pub async fn login(      │
│   tests/auth_test.rs    +40 -0 │ +    Json(req): Json<..>, │
│   Cargo.toml            +0 -20 │ +) -> Result<..> {        │
│                                │ +    let user = db.find.. │
│                                │                           │
│ ↑↓ Files │ j/k Scroll │ PgUp/PgDn │ Esc: Close            │
└────────────────────────────────┴───────────────────────────┘
```

- **左栏** (35%): 文件列表，path + A/M/D 标记 + +N/-M 统计，选中高亮
- **右栏** (65%): 选中文件的 diff 内容
  - 绿色: 新增行 (+)
  - 红色: 删除行 (-)
  - 蓝色: hunk header (@@...@@)
  - 灰色: 上下文行
- 两栏均有滚动条

### 键盘交互

| 快捷键 | 功能 |
|--------|------|
| ↑/↓ | 文件列表上下选择 |
| j/k | diff 内容逐行滚动 |
| PgUp/PgDn | diff 内容翻页 |
| Home/End | diff 跳到顶部/底部 |
| Esc | 关闭 diff popup |

### 鼠标/触控板

- 文件列表区域滚动：切换选中文件
- Diff 内容区域滚动：浏览 diff 内容
- 两栏独立滚动

### Ticket 列表集成

- Working/Done/Error ticket 的 action bar 显示 `[f] Files`
- Queued ticket 不显示 [f]

## 缓存时机

在 `OrchestrationEngine::report_iteration()` 中，当 ticket 状态变为 Done/Error 时：
1. 获取 assigned_worker → 构造 worktree path
2. 运行 git diff 获取完整 diff
3. 解析 file summary (path, status, additions, deletions)
4. 存入 `ticket_diffs` 表
