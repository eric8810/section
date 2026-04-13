# Issue #001: Sync 架构重构 — 职责分层

## Status

`draft`

## Summary

当前 `sync.rs` 同时承担了快照采集、版本判定、冲突决策、文件传输、事件落库五个职责，全部逻辑混在一个函数链里。这导致：
- 无法独立优化某一层（比如给本地扫描加 mtime 缓存、给远程扫描换成 metadata diff）
- 很难为状态机写纯单元测试，只能依赖控制面集成测试兜底
- OpenDAL 被当成纯 I/O 适配层使用，其 Layer 生态（重试、限流、Tracing）没有发挥价值

## Background

### 当前结构

```
sync_source()
  ├── scan_local_tree()      — 递归遍历 + SHA-256 全量计算
  ├── scan_remote_tree()     — OpenDAL list 全量拉取
  ├── reconcile_path()       — 三向对比 + 冲突判定 + 传输决策
  │     ├── push_local_entry()   — OpenDAL write
  │     ├── pull_remote_entry()  — OpenDAL read
  │     └── delete_*_entry()     — OpenDAL delete
  └── upsert_path_sync_state() — SQLite 持久化
```

所有步骤串行执行，变更检测、传输、状态管理耦合在一起。

### 问题

1. **快照采集和决策耦合** — `reconcile_path()` 既判断状态，又直接执行 pull/push/delete
2. **变更检测没有缓存** — 每轮都全量读文件算 SHA-256，即使文件没变
3. **远程版本探测和传输耦合** — 扫描阶段就可能触发远程读，无法替换为 metadata-only 策略
4. **传输没有并行和观测点** — 所有文件串行执行，重试/限流/日志都没有明确挂载点
5. **缺少分层测试边界** — 想测冲突状态机，必须把本地文件、远程 Operator、SQLite 一起带上

## Proposal

将 sync 拆分为三层，每层有明确的输入输出契约：

```
┌──────────────────────────────────────────┐
│  Sync Coordinator (纯规划层)               │
│  职责: 状态机、冲突策略、事件规划          │
│  输入: Vec<PathSyncInput>                 │
│  输出: SyncPlan + PlannedEvent            │
├──────────────────────────────────────────┤
│  Snapshot Collector / Change Detector     │
│  职责: 采集本地/远程事实并构造成规划输入    │
│  输入: 本地根目录 + Operator + 上次状态     │
│  输出: Vec<PathSyncInput>                 │
├──────────────────────────────────────────┤
│  Transport Layer (基于 OpenDAL + Layers)  │
│  职责: 高效可靠地执行 plan 中的 I/O         │
│  输入: SyncPlan                           │
│  输出: TransportResult                    │
└──────────────────────────────────────────┘
```

### Layer 1: Snapshot Collector / Change Detector

```rust
struct ObservedEntry {
    kind: EntryKind,
    version: Option<String>,
    size: Option<u64>,
    mtime_ms: Option<i64>,
}

struct PathSyncInput {
    path: String,
    previous: Option<PathSyncStateRecord>,
    local: Option<ObservedEntry>,
    remote: Option<ObservedEntry>,
}

trait SnapshotCollector {
    fn collect_local(
        &self,
        root: &Path,
        previous: &HashMap<String, PathSyncStateRecord>,
    ) -> Result<HashMap<String, ObservedEntry>>;

    fn collect_remote(
        &self,
        op: &Operator,
        previous: &HashMap<String, PathSyncStateRecord>,
    ) -> Result<HashMap<String, ObservedEntry>>;

    fn build_inputs(
        &self,
        previous: HashMap<String, PathSyncStateRecord>,
        local: HashMap<String, ObservedEntry>,
        remote: HashMap<String, ObservedEntry>,
    ) -> Vec<PathSyncInput>;
}
```

说明：
- `PathSyncInput` 必须保留 `previous/local/remote` 三组事实，不能只剩 `added/modified/deleted` 的路径列表
- `ObservedEntry` 允许 collector 用不同策略求 `version`，例如本地 `mtime+size` 快速路径、远程 `etag/last_modified`
- Collector 负责“发现事实”，不负责“决定 pull 还是 push”

### Layer 2: Sync Coordinator

将现有 `reconcile_path` 的状态机逻辑改造成纯规划函数，输入为 `PathSyncInput`，输出为单路径计划：

```rust
enum PlannedOp {
    PullFile { path: String },
    PushFile { path: String },
    CreateLocalDir { path: String },
    CreateRemoteDir { path: String },
    DeleteLocal { path: String, kind: EntryKind },
    DeleteRemote { path: String, kind: EntryKind },
    MarkConflict { path: String },
}

struct PlannedPathResult {
    next_record: Option<PathSyncStateRecord>,
    ops: Vec<PlannedOp>,
    events: Vec<PlannedEvent>,
}
```

职责清晰：
- 只管冲突判定和策略选择
- 读取 `previous/local/remote` 事实，输出 `SyncPlan`
- 不直接做文件 I/O，不直接写 SQLite

### Layer 3: Transport

```rust
trait Transport {
    /// 执行 SyncPlan 中的 I/O 操作，返回每个 path 的执行结果
    fn execute(&self, plan: SyncPlan) -> Result<TransportResult>;
}
```

基于 OpenDAL Operator + Layers：
- `ConcurrentLimitLayer` — 控制并发数
- `RetryLayer` — 失败重试
- `TracingLayer` — 传输日志
- 需要时再补充分片上传能力，但不把“大文件 delta 传输”塞进本 issue

## Acceptance Criteria

- [ ] `SnapshotCollector` 从 `sync.rs` 中抽离，且其输出为 `PathSyncInput { previous, local, remote }`
- [ ] `SyncCoordinator` 不直接调用 `Operator` 或本地文件系统，只负责从 `PathSyncInput` 生成 `SyncPlan`
- [ ] `Transport` 负责执行 `SyncPlan` 的 I/O，并作为 OpenDAL Layers 的唯一挂载点
- [ ] 分层单元测试至少覆盖：同文件双改冲突、删除/修改冲突、文件/目录类型冲突、首次同步
- [ ] 现有控制面测试全部通过，用户可见行为不变
- [ ] Layer 配置可观测：至少有并发数、重试次数、单路径执行耗时日志或指标

## Non-Goals Clarification

- 本 issue 不要求“远程真增量检测”一次完成；Collector 先支持正确的 metadata scan，再由 Issue #002 继续优化
- 本 issue 不要求 watch/event-driven sync；这里只定义分层边界，事件驱动单开 issue

## Out of Scope

- 分块增量同步（rsync delta transfer）— 复杂度高，单开 issue
- 本地 inotify/FSEvents 事件驱动 — 单开 issue
- 远程 S3 事件通知 — 单开 issue
- FUSE 实时挂载层 — 不在本 issue 范围

## References

- `crates/sectiond/src/sync.rs` — 当前实现
- `docs/ARCHITECTURE.md` — 现有架构文档
- `docs/SYNC_MODEL.md` — 同步模型契约
