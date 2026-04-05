# Section Product Model

## 一句话定义

Section 是一个**共享文件系统协作层**：让 agent、人类和普通 shell / editor / script 在同一棵挂载命名空间上协作，而不是分别走不同的访问模型。

## 产品承诺

Section 最终想提供的不是“很多访问方式”，而是**同一种工作方式**：

- 人类看到的是文件和目录
- Agent 看到的也是文件和目录
- bash / python / node / editor 插件操作的仍然是同一棵树
- 底层可以是 S3 / WebDAV / 本地磁盘 / 其他 OpenDAL backend

### 这意味着什么

1. `FS` 是主交互面
2. `CLI / API` 是控制面
3. 平台差异只能存在于挂载 adapter，而不能改变上层协作心智

## 用户与交互面

| 用户 / 进程 | 主访问方式 | 说明 |
|-------------|------------|------|
| AI Agent | 挂载路径 | 与人类共用同一棵 namespace |
| 人类开发者 / 运维 | Finder / shell / editor / 挂载路径 | 主心智模型是普通文件系统 |
| bash / python / node / 普通程序 | 挂载路径 | `execute / scripting` 应主要发生在共享工作树上 |
| CLI / GUI / API | 控制面 | 管理 source、查看状态、诊断、refresh |

## 交互面分层

### Data Plane

这是产品真正的工作面：

- mount path
- 普通文件系统语义
- 读 / 写 / 列目录 / rename / delete / exec
- shell / script / editor / agent 全部在这一层工作

### Control Plane

这是产品的管理面：

- source add/remove/list
- status / diagnostics
- refresh / health
- 配置、认证、安装引导
- 未挂载平台上的 fallback

CLI / API 仍然重要，但不应成为最终协作模式的替代品。

## 目标架构

### sectiond

产品目标中的核心不是当前的 `section-cli` 或 `section-fuse`，而是一个长期驻留的本地核心，例如 `sectiond`。

它应该统一持有：

- source registry
- OpenDAL operator 生命周期
- metadata/content cache
- refresh / invalidation
- permission / conflict semantics
- health / diagnostics

这样：

- Linux mount adapter 消费它
- macOS mount adapter 消费它
- CLI / API / GUI backend 也消费它

### 挂载 adapter

Section 的平台差异应该被限制在 adapter 层。

#### Linux

- FUSE 作为正式 data-plane adapter
- Linux 是第一参考实现

#### macOS

短期：

- 用 macFUSE adapter 跑通与 Linux 同类的挂载协作语义

长期：

- 如果 macFUSE 的安装摩擦不可接受，再评估 native adapter

关键点是：不管 adapter 怎么变，上层都必须还是“同一 FS 协作模式”。

## 执行与脚本

如果目标是“agent 和人类共用同一个 FS 介质”，那 `execute / scripting` 不应该主要依赖专门的 CLI 命令。

正确的方向是：

- 普通 bash / python / node 脚本直接运行在 mounted workspace 上
- `section exec` 保留，但只作为补充工具
- 产品语义围绕“共享工作树”而不是“专有命令集”

## 平台策略

| 能力 | Linux | macOS | 说明 |
|------|-------|-------|------|
| `core/provider/CLI` | 正式支持 | 正式支持 | non-mount 路径应持续保持 green |
| 挂载协作模型 | 正式实现 | 短期通过 macFUSE，长期再评估 | 上层协作语义必须一致 |
| shell / script 工作流 | 以挂载树为主 | 以挂载树为目标 | CLI fallback 不是终局 |

## 当前 repo truth

当前仓库仍是 pre-sectiond 状态。

已经证明的部分：

- Linux FUSE happy path 可行
- S3 / WebDAV / fs 验证路径已建立
- 双平台 non-FUSE CI 已稳定

还没完成的关键变化：

- 共享语义尚未收拢到 `sectiond`
- CLI 和 mount adapter 还没有共同消费一个统一内核
- macOS mount adapter 仍未做实

## MVP 定义（更新后）

新的 MVP 不再定义为“若干命令都能跑”，而是：

1. Linux 上已经有真实可用的共享 FS 协作路径
2. `sectiond` 作为统一本地核心开始成形
3. CLI 明确转向 control plane
4. macOS 路径有诚实的短期策略，而不是口头等价

## 明确不做

下面这些不应当被当成当前主路线：

- 把 CLI/API-only 当成最终产品
- 用“多入口访问”替代“共享 FS 协作介质”
- 为了零依赖包装，提前重写整个 macOS 平台实现
