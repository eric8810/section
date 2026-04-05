# Section Architecture

## 目标

Section 的最终目标不是“再做一个多存储 CLI”，而是提供一个**共享文件系统协作介质**：

- 人类通过 Finder / shell / editor / 普通程序访问
- Agent 通过相同的路径、相同的文件操作、相同的 shell 工具访问
- 双方在**同一棵命名空间**上协作，而不是一边走挂载，一边走专有 API

因此，Section 的核心不是 CLI，也不是某个单独的 FUSE 进程，而是：

1. 一个统一的本地数据平面
2. 一套稳定的挂载语义
3. 一个为挂载、CLI、API 同时服务的本地控制平面

## 设计原则

### 1. FS 是主交互面，不是附属功能

对最终用户心智来说，Section 应该首先表现为：

- 一棵可挂载的统一命名空间
- 一组普通文件 / 目录
- 一种 shell / editor / script 可直接操作的工作介质

CLI / API 依然重要，但它们是：

- source 管理
- 认证与配置
- health / status / diagnostics
- refresh / cache control
- 非挂载 fallback

而不是主要协作面。

### 2. CLI / API 属于 control plane

如果最终模型是“所有 agent 和人类都在同一个 FS 介质上协作”，那么 CLI / API 的职责应该收敛到控制面：

- `section source add/remove/list`
- `section status`
- `section refresh`
- 配置、认证、诊断
- 在未挂载平台上的临时 fallback

`ls/cat/cp/write/exec` 这些 CLI 能力可以保留，但它们不应该定义产品的终局形态。

### 3. 共享语义必须在一个长期驻留的本地核心里实现

缓存、refresh、权限、路由、连接状态、冲突处理，不应该散落在：

- `section-cli`
- `section-fuse`
- 将来的 GUI / API backend

里各写一份。

正确的方向是引入一个长期驻留的本地核心进程，例如 `sectiond`，作为唯一真实状态机。

`sectiond` 的第一版边界定义见 [SECTIOND.md](./SECTIOND.md)。

## 目标运行时模型

```text
            Humans / Agents / Shell / Editors
                         |
          +--------------+--------------+
          |                             |
      Control Plane                 Data Plane
  (CLI / API / GUI backend)    (mounted shared namespace)
          |                             |
          +-------------+---------------+
                        |
                     sectiond
      source registry / routing / cache / refresh
      permissions / health / sessions / diagnostics
                        |
        +---------------+------------------+
        |                                  |
   Linux mount adapter                macOS mount adapter
      (FUSE today)                 (macFUSE first, native later)
                        |
                     OpenDAL
              S3 / WebDAV / fs / ...
```

## 模块职责

### sectiond

目标中的 `sectiond` 负责：

- 维护 source registry
- 管理后端 operator 生命周期
- 管理 metadata/content cache
- 实现 refresh / invalidation
- 统一 permission / conflict / health 语义
- 为 CLI / mount adapter / GUI backend 提供统一本地接口

这是未来真正的产品内核。

### section-cli

`section-cli` 在目标架构中的职责是 control plane client：

- 管理 source
- 显示状态 / 健康 / 诊断
- 触发 refresh / repair
- 在未挂载平台上提供有限但诚实的 fallback

它不再承担“产品的主要工作面”这个角色。

### Linux mount adapter

Linux 路径继续沿用 FUSE，并且是最先做实的正式 data-plane adapter。

Linux adapter 的职责：

- 把 `sectiond` 暴露成 POSIX 文件系统
- 保证 shell / editor / agent 的工作路径是真正的 mount path
- 为跨平台语义提供第一条已验证的参考实现

### macOS mount adapter

macOS 的长期要求不是“最好也能 mount”，而是**最终也要给出同一类 FS 协作心智**。

短期路线：

- 先用 macFUSE 把语义跑通
- 把它视为 macOS 上的过渡 adapter

长期路线：

- 如果 macFUSE 安装摩擦不可接受，再评估是否需要原生 adapter
- 但那时也应该只是“替换 adapter”，而不是推翻上层语义

## 执行与脚本模型

既然目标是“人类与 agent 在相同 FS 介质上协作”，那么：

- bash / python / node / editor macro 等执行模式，都应该主要针对挂载路径工作
- `section exec` 最多是辅助能力，不该成为核心工作流

正确的协作模式应该是：

- 人类在 `/mnt/section/...` 或平台等价挂载点里工作
- Agent 在相同路径上读写
- 普通 shell 脚本直接运行在这棵树上
- `sectiond` 负责缓存、一致性与 refresh 语义

## 平台策略

### Linux

- 正式 data-plane 平台
- 挂载路径应持续保持可验证、可诊断、可脚本化
- Linux FUSE 语义是第一参考实现

### macOS

- non-mount CLI / core / provider 路径要保持可用
- mount 路径短期通过 macFUSE 做实
- 如果用户强需求是“零额外依赖 + 文件系统体验”，应评估 native adapter，而不是让 CLI/API 变成终局替代品

## 当前仓库与目标架构的关系

当前 repo 仍处于**pre-sectiond** 阶段。

已经完成的部分：

- OpenDAL 多后端接入
- provider store / credential encryption
- Linux FUSE happy-path validation
- 双平台 non-FUSE CI
- 基础 CLI 与缓存/refresh 机制

还没有完成的关键变化：

- 将 `section-cli` / `section-fuse` 的核心状态与逻辑从“各自直接持有”迁移到 `sectiond`
- 让控制面与数据面都消费同一套本地状态机
- 明确 macOS adapter 的短期与长期策略

## 非目标

下面这些都不应成为当前阶段的主路线：

- 把 CLI/API-only 当成最终产品模型
- 为了追求 macOS 零依赖，一开始就重做整套产品
- 让 Linux 和 macOS 在不同的交互心智下“都算支持”

如果支持不是同一套 FS 协作模式，那只是多入口访问，而不是 Section 想要的共享介质。
