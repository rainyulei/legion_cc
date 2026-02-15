# Squad Board 四项修复设计

## Fix 1: Log 从 worker level 改为 ticket level

**根因:** `sdk_log_buffer` 挂在 Pane（worker）上，新 ticket 复用老 ticket 的 buffer 导致串台。

**方案:**
- App 新增 `ticket_logs: HashMap<usize, Arc<Mutex<Vec<String>>>>` — key=ticket_id
- `start_sdk_task()`: iteration==1 创建新 buffer 放入 map; iteration>1 从 map 取已有 buffer
- `draw_board_detail_popup()` 根据 ticket.id 从 ticket_logs 读取
- Pane.sdk_log_buffer 保留为当前运行任务的引用（SDK spawn 仍需要）
- ticket 清理时同步清理 map

## Fix 2: 进入看板自动激活第一个 ticket

**根因:** `board_selected` 初始值 0，ticket id 从 1 开始。

**方案:** 渲染 Board 时检查：如果 board_selected 不在当前 ticket 列表中，自动选第一个。

## Fix 3: Task 清理

**方案:**
- `d` 键删除选中的 Done/Error ticket
- `D` (Shift+d) 批量清理所有 Done+Error
- OrchestrateEngine 新增 `delete_ticket(id)` 和 `clear_completed()`
- DB 层新增 DELETE 操作
- 清理时同步清理 ticket_logs map

## Fix 4: Worker prompt 增强

**A. effective prompt 结构化:**
```
# Task: {title}
## Context
{context}
## Success Criteria
{criteria}
## Task Description
{prompt}
```

需要从 engine 传入 title/context/criteria 到 start_sdk_task()。

**B. worker_instructions() 增强:**
- 明确 headless 非交互模式
- TechLeader: 分析→创建 task list→分配
- Engineer: TDD 实现
- QA: 验收测试
- 简化流程，明确 promise 触发条件
