# Section Architecture

## 目标

Section 的主目标从“挂载统一命名空间”调整为：

> 提供一个跨平台、可靠、可物化的 sync workspace，让人类和 agent 在同一份本地工作区上协作。

这意味着：

- `workspace` 是主工作面
- `sectiond` 是真实状态机
- `runtime` 单独负责执行一致性
- mount / native integration 变成后续 adapter，而不是主线前提

## 顶层原则

### 1. Workspace first

最终用户首先应该看到的是：

- 一个本地目录
- 文件确实落在本地时可直接使用
- 可以知道哪些内容已物化、哪些待同步、哪些冲突

而不是：

- 一组专有 CLI 命令
- 一套必须先装驱动才能用的 mount 路径

### 2. sectiond 是唯一真实状态机

缓存、同步、refresh、冲突、materialization、source 生命周期，不应散落在：

- `section-cli`
- 未来 GUI
- 未来 adapter

而应统一放进 `sectiond`。

### 3. Execution 不是 FS 层承诺

Section 不尝试用文件系统方案统一：

- POSIX shell 行为
- Windows 原生执行行为
- 所有平台的权限与 metadata 语义

Section 统一 workspace contract；
execution contract 由 runtime 层承接。

## 目标运行时模型

```text
 Humans / Agents / Shell / Editors
                |
         Local Sync Workspace
                |
            sectiond core
 source registry / sync state / materialization
 local change ingest / remote change ingest / conflicts
                |
       +--------+--------+
       |                 |
   Control Plane     Future Adapters
 (CLI / GUI / API)  (FUSE / File Provider /
                     CFAPI / SMB export)
                |
             OpenDAL
      S3 / WebDAV / fs / ...
```

## 模块职责

### sectiond

`sectiond` 目标职责：

- source registry
- workspace registry
- remote operator 生命周期
- local materialization 状态
- sync scheduler
- metadata/content cache
- local change detection ingestion
- remote change reconciliation
- conflict detection and surfacing
- health / diagnostics

它是产品的核心，不再只是 mount 支撑组件。

### section-cli

CLI 的目标职责：

- source / workspace 管理
- status / health / diagnostics
- sync / materialize / pin / repair
- conflict inspection
- 在无 GUI 时充当控制面入口

CLI 不再是“产品本体”，而是 control plane client。

### Local Workspace

本地 workspace 是主工作面。

它应该支持：

- 普通文件/目录读写
- 部分内容未物化时的诚实状态
- 本地修改可被检测和同步
- agent 与人类共享同一目录

第一阶段不要求：

- 完整 POSIX metadata round-trip
- 任意复杂文件类型完整支持
- 平台原生 mount 语义

### Future Adapters

这些都不删，但全部降级为后续 adapter：

- Linux `FUSE`
- macOS `macFUSE`
- Windows `WinFsp`
- macOS File Provider
- Windows CFAPI
- SMB export

它们应该消费 `sectiond` 的 workspace contract，而不是定义主产品。

## 核心状态模型

### Object State

每个 workspace object 至少要区分：

- present and materialized
- present but not materialized
- local dirty
- remote newer
- in conflict
- failed / needs repair

### Workspace State

每个 workspace 至少要区分：

- healthy
- syncing
- offline
- degraded
- conflict present

## 核心数据流

### Remote to Local

1. remote source emits or is polled for change
2. `sectiond` updates object state
3. object is materialized or marked stale
4. workspace exposes the current truthful local state

### Local to Remote

1. user/agent edits local file
2. local change is observed
3. `sectiond` stages sync work
4. remote write succeeds or conflict is surfaced

### Execution

1. runtime points at local workspace
2. Section guarantees readiness/materialization contract only
3. runtime owns interpreter/container/OS-specific execution behavior

## 为什么这样比 FUSE-first 更合理

因为它先解决：

- 普通用户真的能用
- agent 和人类能共享本地目录
- 跨平台可交付

而把下面这些留到第二阶段：

- mount fidelity
- native shell integration
- deeper OS-specific file presentation

## 当前 repo 与目标架构的关系

已经可复用的 groundwork：

- OpenDAL backend 接入
- provider store / credential encryption
- cache / refresh 基础能力
- `sectiond` 初始边界
- Linux `FUSE` 验证经验
- 双平台 non-FUSE CI

仍缺的主线能力：

- workspace registry
- materialization state model
- local change ingestion
- bidirectional sync loop
- conflict surfacing
- workspace-oriented CLI

## 非目标

当前 pivot 后，不再把下面这些当主线目标：

- “所有平台都先 mount 再说”
- “先统一 POSIX，再统一产品”
- “靠一个 FS 方案同时统一工作区和执行语义”
