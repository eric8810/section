# Section Route Map

## 新方向结论

Section 现在从 `FS-first mount product` pivot 到：

> `sync workspace first` 产品主线

更直接地说：

- **主线**：本地 workspace + sync/materialize/conflict
- **执行**：靠 runtime contract
- **高级模式**：FUSE / native integration / SMB export

`FUSE` 不再是普通用户默认入口。

## 为什么 pivot

原因已经足够明确：

1. 普通用户使用 `FUSE` 成本过高
2. mount 路线并不能顺便统一 Windows 的原生执行语义
3. 真正需要统一的是 workspace，不是宿主机 POSIX

## 当前已完成的 groundwork

这些工作仍然保留：

- OpenDAL 多 backend 接入
- provider store / credential encryption
- cache / refresh 基础能力
- 基础 CLI
- 双平台 non-FUSE CI
- Linux FUSE happy path 验证
- `sectiond` 初始边界

不再作为主线的内容：

- macOS mount adapter 验证
- FUSE-first 路线图
- 以挂载路径作为默认工作面

## 新主路线

### Phase 1: 定义 sync workspace contract

目标：

- 定清楚 workspace 是什么
- 定清楚保什么、不保什么
- 定清楚 agent / 人类如何共享同一份本地目录

产出：

- workspace object model
- metadata scope / non-goals
- readiness / materialize / pin model
- conflict model

### Phase 2: sectiond 变成 sync core

目标：

- 让 `sectiond` 从 mount-centric boundary 变成 workspace sync core

产出：

- workspace registry
- sync state store
- materialization state machine
- sync scheduler

### Phase 3: 打通双向同步

目标：

- remote -> local
- local -> remote
- 本地修改、远端修改、冲突、修复都可见

产出：

- local change ingest
- remote change ingest
- bidirectional reconciliation
- conflict surfacing

### Phase 4: 重写 control plane

目标：

- CLI/GUI/API 不再围绕 mount，而围绕 workspace

产出：

- workspace create/list/status
- sync / pin / materialize / repair
- conflict inspection

### Phase 5: 定义 execution contract

目标：

- 承认 workspace 与 execution 分层
- 明确 agent/script 如何在 workspace 上工作

产出：

- runtime assumptions
- POSIX-only workloads 的边界
- Windows 的执行策略

### Phase 6: 评估后续 adapter / native integration

目标：

- 在 workspace contract 稳定后，再评估：
  - FUSE advanced mode
  - macOS File Provider
  - Windows CFAPI
  - SMB export

产出：

- secondary tracks，而不是主线前提

## 建议 issue map

下面这些是 pivot 后应该优先开的主 issue：

| 阶段 | 主题 | GitHub issue |
|------|------|--------------|
| Phase 1 | 定义 sync workspace product contract 与非目标 | `TBD` |
| Phase 2 | 将 `sectiond` 重定义为 workspace sync core | `TBD` |
| Phase 3 | 实现 local materialization / sync state / readiness model | `TBD` |
| Phase 3 | 实现本地变更检测、远端变更收敛与 conflict surfacing | `TBD` |
| Phase 4 | 将 CLI 重构为 workspace control plane | `TBD` |
| Phase 5 | 定义 execution contract（runtime、materialize、Windows 边界） | `TBD` |
| Phase 6 | 评估 future adapters：FUSE / File Provider / CFAPI / SMB | `TBD` |

## 建议执行顺序

1. 先定 `workspace contract`
2. 再定 `sectiond sync core`
3. 再做双向同步与冲突
4. 再做 workspace-oriented CLI
5. 再定 execution contract
6. 最后才讨论 mount / native integration / SMB

## 当前 repo truth

截至当前：

- repo 已有不少可复用基础能力
- 但 repo 的主叙事仍是 pre-pivot
- 真正缺的是：把现有能力收拢成一个 `sync workspace` 产品，而不是继续在 mount 路线上打磨

## 不再建议的路线

下面这些不再建议作为主战场：

1. 继续把 `FUSE` 当普通用户默认入口
2. 继续把 macOS mount 验证当当前最重要 blocker
3. 继续试图用一个文件系统方案直接统一跨平台执行语义
