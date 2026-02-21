# Checkpoint & DAG Enhancement Design

## Problem

1. **DAG 不检查 merge 状态** — `is_ready()` 只看 `TicketStatus::Done`，不看 `MergeStatus::Merged`。下游 ticket 可能在上游代码还没合并到 leader 时就开始了，导致 worker 拿不到上游的代码变更。
2. **Worker 看不到上游做了什么** — 下游 worker 只能看到自己 ticket 的 prompt，不知道前序 ticket 改了哪些文件、写了什么代码、遇到了什么问题。
3. **Checkpoint ticket 没有特殊处理** — 和普通 ticket 一样执行，无法在发现前序未 merge 时主动修复。

## Design

### 1. DAG 调度：检查 merge 状态

**`is_ready()` 分两种逻辑：**

- **普通 ticket**：`blocked_by` 中的所有 ticket 必须 `Done + Merged`（或 `Skipped`）。如果有 `Done` 但 `Pending/Conflict` 的 → 不释放，ticket 保持 Queued。
- **Checkpoint ticket**（`is_checkpoint == true`）：`blocked_by` 中有未 Merged 的 ticket → 仍然释放，但 checkpoint worker 的职责包括检查并修复 merge 问题。

**需要在 `TaskTicket` 上加 `is_checkpoint: bool` 字段。**

dispatch 时通过 `--checkpoint` flag 设置。

### 2. MCP 工具：`legion-deps`

新增 MCP 工具让 worker 查询前序 ticket 的信息。

**输入**: ticket_id（可选，默认查当前 ticket 的所有 blocked_by）

**输出**:
```json
{
  "dependencies": [
    {
      "id": 1,
      "title": "Implement DB schema",
      "status": "done",
      "merge_status": "merged",
      "summary": "Created User, Post, Comment tables...",
      "diff_summary": [
        {"path": "src/db/schema.rs", "status": "added", "additions": 120, "deletions": 0},
        {"path": "src/db/mod.rs", "status": "modified", "additions": 5, "deletions": 0}
      ],
      "log_tail": "... last 50 lines of worker execution log ..."
    }
  ]
}
```

数据来源：
- `summary` — `tickets` 表的 `summary` 字段
- `diff_summary` — `ticket_diffs` 表的 `file_summary` 字段
- `log_tail` — `ticket_logs` 表或内存中的 `app.ticket_logs`

### 3. Worker Prompt 增强

**普通 ticket 的 prompt 增加：**
```
## Dependency Check
Before starting, verify your upstream dependencies are merged:
- Use `legion-deps` to check the status and content of tickets you depend on
- If any dependency shows merge_status != "merged", report via legion-check
```

**Checkpoint ticket 的 prompt 增加：**
```
## Checkpoint: Verify & Fix Integration

You are a checkpoint ticket. Your job:
1. Use `legion-deps` to check all upstream tickets' merge status
2. For any unmerged tickets: investigate why, attempt to merge/fix
3. Run build + test + lint on the current codebase
4. Fix any integration issues found
5. When everything passes, output <promise>DONE</promise>
```

### 4. Engine Changes

**`TaskTicket` 新增字段：**
```rust
pub is_checkpoint: bool,
```

**`is_ready()` 修改：**
```rust
fn is_ready(ticket: &TaskTicket, all_tickets: &[TaskTicket]) -> bool {
    ticket.blocked_by.iter().all(|dep_id| {
        all_tickets.iter()
            .find(|t| t.id == *dep_id)
            .map(|t| {
                if ticket.is_checkpoint {
                    // Checkpoint: only needs Done (will handle merge itself)
                    t.status == TicketStatus::Done
                } else {
                    // Normal: needs Done + Merged
                    t.status == TicketStatus::Done
                        && matches!(t.merge_status, MergeStatus::Merged | MergeStatus::Skipped)
                }
            })
            .unwrap_or(true)
    })
}
```

**DB schema 变更：**
- `tickets` 表加 `is_checkpoint INTEGER DEFAULT 0`

**`submit_ticket()` 参数加 `is_checkpoint: bool`**

### 5. MCP Tool dispatch 增强

`legion-dispatch` 增加 `--checkpoint` flag：
```bash
legion-dispatch 1 -t "Verify DB module" --checkpoint --after 1,2 "Run build, test, lint..."
```

## Files to Modify

| File | Changes |
|------|---------|
| `crates/legion-core/src/orchestrate/engine.rs` | `TaskTicket.is_checkpoint`, `is_ready()` 逻辑, `submit_ticket()` 参数 |
| `crates/legion-db/src/lib.rs` | `TicketRow.is_checkpoint`, schema migration |
| `crates/legion-tools/src/lib.rs` | `legion-dispatch` 增加 `--checkpoint`, 新增 `legion-deps` 工具 |
| `crates/legion-tui/src/claudemd.rs` | Worker prompt 增加 dependency check 指导 |
| `crates/legion-tui/src/app.rs` | `start_sdk_task` 传递 checkpoint 信息 |
