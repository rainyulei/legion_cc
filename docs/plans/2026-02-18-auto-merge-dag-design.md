# Auto-Merge + Task DAG Design

## Goal

Worker 完成任务后自动将代码 merge 到 leader branch，并通过 DAG 依赖控制任务执行顺序，确保有依赖关系的任务串行执行、无依赖的并行执行。

## Architecture

```
用户 → Leader (Claude Code)
         │
         ├─ legion-dispatch 1 -t "Auth API" ...
         ├─ legion-dispatch 2 -t "Auth UI" --after 1 ...
         └─ legion-dispatch 3 -t "Tests" --after 1,2 ...

Engine 调度:
  ┌─ Ticket 1 (无依赖) → 立即分配 Worker-1
  │    Worker-1 完成 → commit → auto-merge 到 leader branch
  │
  ├─ Ticket 2 (after: 1) → Ticket 1 Done 后分配
  │    分配前: worker worktree rebase 到 leader 最新 (含 Ticket 1 代码)
  │    Worker 完成 → commit → auto-merge 到 leader branch
  │
  └─ Ticket 3 (after: 1,2) → Ticket 1+2 都 Done 后分配
       分配前: worker worktree rebase 到 leader 最新 (含 1+2 代码)
       Worker 完成 → commit → auto-merge 到 leader branch
```

## Data Model Changes

### TaskTicket 新增字段

```rust
pub struct TaskTicket {
    // ... existing fields ...
    pub blocked_by: Vec<usize>,      // 依赖的 ticket IDs
    pub merge_status: MergeStatus,   // 合并状态
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeStatus {
    Pending,    // 还没 merge（Working/Queued 状态）
    Merged,     // 成功 merge 到 leader
    Conflict,   // merge 冲突，已 abort
    Skipped,    // 没有需要 merge 的内容
}
```

### DB Schema

```sql
-- TicketRow 新增列
blocked_by TEXT DEFAULT '[]'       -- JSON array, e.g. "[1, 3]"
merge_status TEXT DEFAULT 'pending' -- "pending" / "merged" / "conflict" / "skipped"
```

## DAG Scheduling

### is_ready() 检查

```rust
fn is_ready(&self, ticket: &TaskTicket) -> bool {
    ticket.blocked_by.iter().all(|dep_id| {
        self.tickets.iter()
            .find(|t| t.id == *dep_id)
            .map(|t| t.status == TicketStatus::Done)
            .unwrap_or(true)  // 依赖不存在视为已完成
    })
}
```

### take_next() 变更

现有逻辑找第一个 Queued ticket，改为找第一个 Queued 且 `is_ready()` 的 ticket。

### 循环依赖检测

dispatch 时检测循环依赖，拒绝并返回错误。

## Auto-Merge Flow

### Worker 完成时（lib.rs event loop）

```
Worker Done (promise found)
  │
  ├─ 1. report_iteration(ticket_id, true, summary)
  ├─ 2. Cache diff (在 worker worktree 上，此时还是原始状态)
  ├─ 3. Auto-merge (在 leader worktree 上，不影响 worker)
  │     a. leader 有未提交改动 → git stash
  │     b. git merge <worker-branch> --no-ff
  │     c. 成功 → merge_status = Merged
  │     d. 冲突 → git merge --abort → merge_status = Conflict
  │     e. leader 有 stash → git stash pop
  ├─ 4. Clean up SDK task
  └─ 5. 取下一个任务时 → rebase worker worktree
```

关键：diff cache 必须在 auto-merge 之前，因为 merge 在 leader worktree 操作不影响 worker worktree，但后续 rebase-on-start 会改变 worker worktree 的 HEAD。

### Worker 开始新任务前（Rebase-on-start）

```
take_next() 返回新 ticket
  │
  ├─ 1. 在 worker worktree 中 git merge <leader-branch>
  │     (把 leader 最新代码拉到 worker worktree)
  │     失败则 git merge --abort + git reset --hard <leader-HEAD>
  ├─ 2. 记录 base_commit = HEAD (merge 后的最新)
  └─ 3. start_sdk_task(...)
```

### Error ticket 处理

- 不自动 merge — 失败的代码可能不完整
- merge_status 保持 Pending
- Worker branch 和 worktree 保留，Leader 可查看 diff
- Leader 可手动 merge 或重新 dispatch

## legion-dispatch 变更

```bash
# 新增 --after 参数（可选）
legion-dispatch <worker_id> -t "title" -c "context" -k "criteria" --after 1,3 "description"
```

Orchestrate API submit endpoint 新增 `blocked_by` 字段：
```json
{
  "title": "Auth UI",
  "ticket": "implement login form...",
  "blocked_by": [1, 3]
}
```

## legion-check 显示变更

```
=== Ticket Queue ===
Total: 3  |  Queued: 1  |  Working: 1  |  Done: 1  |  Error: 0

--- WORKING (1) ---
  [2] "Auth UI" worker=1  (45s)  after: #1 done

--- DONE (1) ---
  [1] "Auth API" worker=2  (120s)  [merged]

--- QUEUED (1) ---
  [3] "Tests"  after: #1 done #2 working
```

## Edge Cases

| 场景 | 处理 |
|------|------|
| 依赖的 ticket 变成 Error | 被阻塞的 ticket 保持 Queued，Leader 决定如何处理 |
| 循环依赖 | dispatch 时检测并拒绝 |
| 依赖不存在的 ticket ID | 视为已完成，不阻塞 |
| Leader worktree 有未提交改动 | merge 前 git stash，merge 后 git stash pop |
| 两个 Worker 同时完成 | 事件循环单线程顺序处理，不会并发 merge |
| Default session (leader = 主仓库) | merge 目标是主仓库当前分支，逻辑一样 |

## Leader CLAUDE.md 更新

在 dispatch 说明中加入 `--after` 用法：
```
legion-dispatch <worker_id> -t "title" -c "context" -k "criteria" [--after 1,3] "description"

--after — 可选，指定依赖的 ticket IDs（逗号分隔）。
         被依赖的 ticket 全部 Done 后才会分配此任务。
         用于有文件依赖的任务，确保串行执行。
```
