# Section Architecture

## 目标

Section 的主目标从“挂载统一命名空间”调整为：

> 提供一个跨平台、可靠、可物化的 source/path sync 层，让人类和 agent 在同一份本地目录上协作。

这意味着：

- `source/path` 是主工作面
- `sectiond` 是真实状态机
- `execution` 不在当前项目范围
- 对外状态保持在 `ready / syncing / conflict / error`

## 顶层原则

### 1. Source/path first

最终用户首先应该看到的是：

- 一个 source 和它的 path 视图
- 某些 source/path 可以落在本地目录里直接使用
- 可以知道当前是 `ready / syncing / conflict / error`

而不是：

- 一组专有 CLI 命令
- 一套必须先装驱动才能用的 mount 路径

### 2. sectiond 是唯一真实状态机

缓存、同步、refresh、冲突、本地状态、source 生命周期，不应散落在：

- `section-cli`
- 未来 GUI
- 未来 adapter

而应统一放进 `sectiond`。

### 3. Execution 不在当前项目范围

Section 当前只统一 source/path sync contract，不处理执行方案。

## 目标运行时模型

```text
 Humans / Agents / Shell / Editors
                |
        Local Source Trees
                |
            sectiond core
 source registry / sync state / local detail
 local change ingest / remote change ingest / conflicts
                |
          Control Plane
       (CLI / GUI / API)
                |
             OpenDAL
      S3 / WebDAV / fs / ...
```

## 模块职责

### sectiond

`sectiond` 目标职责：

- source registry
- source local-root bindings
- remote operator 生命周期
- source/path public state
- source/path detail state
- sync scheduler
- metadata/content cache
- local change detection ingestion
- remote change reconciliation
- conflict detection and surfacing
- health / diagnostics

它是产品的核心，不再只是 mount 支撑组件。

### section-cli

CLI 的目标职责：

- source / path 管理
- status / health / diagnostics
- sync / pull / pin / repair
- conflict inspection
- 在无 GUI 时充当控制面入口

CLI 不再是“产品本体”，而是 control plane client。

### Local Source Tree

本地 source tree 是主工作面，但它不是额外的顶层对象。

它应该支持：

- 普通文件/目录读写
- 简单而诚实的外部状态
- 本地修改可被检测和同步
- agent 与人类共享同一目录

第一阶段不要求：

- 完整 POSIX metadata round-trip
- 任意复杂文件类型完整支持
- 平台原生 mount 语义

## 核心状态模型

### Public Path State

每个 path 对外只区分：

- `ready`
- `syncing`
- `conflict`
- `error`

### Public Source State

每个 source 对外只区分：

- `ready`
- `syncing`
- `conflict`
- `error`

### Detail Fields

内部或详情视图可以再细分：

- `local_present`
- `dirty_local`
- `dirty_remote`
- `pinned`
- `stale`
- health/error reason

## 核心数据流

### Remote to Local

1. remote source emits or is polled for change
2. `sectiond` updates object state
3. object detail is updated and public state stays truthful
4. source/path exposes the current truthful local state

### Local to Remote

1. user/agent edits local file
2. local change is observed
3. `sectiond` stages sync work
4. remote write succeeds, or Section detects that the write is based on stale remote state
5. stale overwrite is blocked and requires explicit `use-local` / `use-remote`

## 为什么这样比 FUSE-first 更合理

因为它先解决：

- 普通用户真的能用
- agent 和人类能共享本地目录
- 跨平台可交付

## 当前 repo 与目标架构的关系

已经可复用的 groundwork：

- OpenDAL backend 接入
- provider store / credential encryption
- cache / refresh 基础能力
- `sectiond` 初始边界
- Linux `FUSE` 验证经验
- 双平台 non-FUSE CI

仍缺的主线能力：

- source local-root binding model
- path public/detail state model
- local change ingestion
- bidirectional sync loop
- conflict surfacing and resolution
- source/path-oriented CLI

## 非目标

当前 pivot 后，不再把下面这些当主线目标：

- “所有平台都先 mount 再说”
- “先统一 POSIX，再统一产品”
- “靠一个 FS 方案同时统一工作区和执行语义”
