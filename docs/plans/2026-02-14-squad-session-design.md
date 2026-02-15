# Squad Session Management Design

## Goal

Squad 模式支持 session 持久化：每个 session 对应一组 git worktrees（Leader + N Workers），启动时可选择恢复已有 session 或创建新 session。任务完成后可手动标记完成并选择合并策略。

## Architecture

```
legion squad --workers 2

启动流程:
1. 显示 Session 选择列表
2. 选择已有 session → 恢复 worktrees，claude --continue
3. 选择 "New Session" → 创建 worktrees，claude 全新启动

运行时 Ctrl+P:
├─ Switch Models      → Provider/Model 选择
├─ Switch Session     → Session 列表（切换/新建）
├─ Complete Session   → 标记完成 + 合并选择
└─ Quit
```

## Worktree 布局

```
~/projects/my-app/                          ← 原始项目 (main branch)
~/projects/my-app-legion/                   ← Legion worktree 根目录
├── fix-auth-bug/                           ← Session "fix-auth-bug"
│   ├── leader/                             ← Leader worktree (branch: legion/fix-auth-bug/leader)
│   ├── worker-1/                           ← Worker 1 worktree (branch: legion/fix-auth-bug/worker-1)
│   └── worker-2/                           ← Worker 2 worktree (branch: legion/fix-auth-bug/worker-2)
└── add-dark-mode/                          ← Session "add-dark-mode"
    ├── leader/
    └── worker-1/
```

- 每个 pane 独立 worktree + 独立 git branch
- Branch 命名: `legion/<session-name>/<pane-label>`
- Worktree 路径: `../<project>-legion/<session-name>/<pane-label>/`
- Claude Code `--continue` 在对应 worktree 目录运行，自然恢复上次会话

## 数据模型

### `squad_sessions` 表

```sql
CREATE TABLE IF NOT EXISTS squad_sessions (
    name TEXT PRIMARY KEY,
    project_path TEXT NOT NULL,
    worker_count INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',   -- active / completed
    created_at INTEGER NOT NULL,
    completed_at INTEGER
);
```

- `name`: session 名称，用户创建时输入（如 "fix-auth-bug"）
- `project_path`: 原始项目路径（用于定位 worktree 根目录）
- `worker_count`: 该 session 的 worker 数量
- `status`: `active` 或 `completed`
- `completed_at`: 完成时间戳

## Session 生命周期

### 创建 (New Session)

1. 用户输入 session 名称
2. 计算 worktree 根路径: `<project_parent>/<project_name>-legion/<session_name>/`
3. 为每个 pane 创建 git worktree:
   ```bash
   git worktree add ../<project>-legion/<session>/leader -b legion/<session>/leader
   git worktree add ../<project>-legion/<session>/worker-1 -b legion/<session>/worker-1
   # ...
   ```
4. 在每个 worktree 目录下启动 `claude`（不带 `--continue`）
5. 写入 DB: `squad_sessions` 记录

### 恢复 (Resume Session)

1. 从 DB 读取 session 信息
2. 验证 worktree 目录存在
3. 按 `worker_count` 创建 pane
4. 在每个 worktree 目录下启动 `claude --continue`
5. Claude Code 自动恢复上次对话上下文

### 切换 (Switch Session)

运行中通过 Ctrl+P → "Switch Session" 切换:

1. 杀掉当前所有 pane 的 PTY 进程
2. 加载目标 session 的 worktree 信息
3. 如果 worker 数量不同，动态调整 pane 数量（增删 pane + proxy + control API）
4. 在目标 session 的 worktree 目录下启动 `claude --continue`
5. 重新渲染 UI

### 完成 (Complete Session)

Ctrl+P → "Complete Session":

1. 弹出确认对话框："Complete session '<name>'?"
2. 确认后弹出合并策略选择:
   - **Merge to main**: 对每个 pane 的 worktree branch 执行 `git merge` 到 main
   - **Keep worktrees**: 仅标记状态为 completed，worktree 保留不动
   - **Discard**: 删除所有 worktree + branch，丢弃变更
3. 执行选择的操作
4. 更新 DB: `status = 'completed'`, `completed_at = now`
5. 自动切换到下一个 active session 或提示创建新 session

### 删除 (Delete Session)

在 session 列表中对 completed 的 session:

1. 确认删除
2. 删除 worktree 目录: `git worktree remove <path>`
3. 删除 git branch: `git branch -D legion/<session>/<pane>`
4. 删除 DB 记录

## Session 列表 UI

```
┌─ Sessions ───────────────────────────────┐
│  ● fix-auth-bug       3 panes   2h ago   │  ← 当前 active (高亮)
│  ○ add-dark-mode      2 panes   1d ago   │  ← 其他 active
│  ✓ refactor-api       4 panes   3d ago   │  ← completed (灰色)
│                                           │
│  [+] New Session                          │
│  [x] Delete (completed only)             │
└───────────────────────────────────────────┘
```

- `●` 当前运行的 session
- `○` 有 worktree 但未激活
- `✓` 已完成（底部显示，灰色）
- Enter 选择恢复/切换
- 在 completed session 上可选 Delete

## Ctrl+P 菜单重构

```
原来 (Matrix 模式):
  Ctrl+P → Matrix (每个 pane 一列，选 Provider/Model)

新增:
  Ctrl+P → Main Menu
            ├─ Switch Models     → 进入 Matrix
            ├─ Switch Session    → Session 列表
            ├─ Complete Session  → 完成确认 + 合并策略
            └─ Quit              → 退出
```

## 端口管理

端口方案不变：
- Leader: proxy = `base_port`, control = `base_port + 1000`
- Worker i: proxy = `base_port + i + 1`, control = `base_port + 1000 + i + 1`

切换 session 时 worker 数量变化：
- 增加 worker → 分配新端口，创建新 proxy + control API
- 减少 worker → 停止多余的 proxy + control API，释放端口

## PTY 启动变更

```rust
// 新增参数
PtyHandle::spawn(
    rows, cols,
    proxy_port, control_port,
    dangerously_skip_permissions,
    worker_id,
    orchestrate_port,
    system_prompt,
    use_proxy,
    working_dir: Option<PathBuf>,  // NEW: worktree 路径
    continue_session: bool,         // NEW: 是否 --continue
)
```

- `working_dir` → `cmd.cwd(path)` 设置 PTY 工作目录
- `continue_session` → 添加 `--continue` 参数

## 合并策略详细

### Merge to main

```bash
# 对每个 pane 的 branch:
cd <project_root>
git checkout main
git merge legion/<session>/leader --no-ff -m "Merge legion session: <session> (leader)"
git merge legion/<session>/worker-1 --no-ff -m "Merge legion session: <session> (worker-1)"
# ...

# 合并成功后清理
git worktree remove <worktree_path>
git branch -d legion/<session>/<pane>
```

如果合并冲突：提示用户手动解决，保持 session 为 active 状态。

### Keep worktrees

仅更新 DB 状态。Worktree 和 branch 保留，用户可以后续手动处理或重新激活。

### Discard

```bash
# 强制删除
git worktree remove --force <worktree_path>
git branch -D legion/<session>/<pane>
rm -rf <worktree_dir>  # 如果 git worktree remove 失败
```

## 启动流程变更

```
legion squad --workers 2

1. 打开 DB
2. 查询 active sessions
3. 如果有 active sessions:
   → 显示 session 选择列表（包含 "New Session" 选项）
   → 用户选择后:
     - 已有 session → resume（claude --continue in worktrees）
     - New Session → 输入名称 → 创建 worktrees → 启动
4. 如果没有 active sessions:
   → 直接提示输入 session 名称 → 创建
```
