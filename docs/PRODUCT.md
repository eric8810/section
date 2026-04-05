# Section Product Model

## 一句话定义

Section 是一个**跨平台 sync workspace 协作层**：

- 把任意后端介质同步/物化到本地工作区
- 让人类、agent、shell、editor 对着同一份本地 workspace 工作
- 不把“统一宿主机 POSIX 执行语义”当成主产品承诺

## 核心产品判断

这次 pivot 的关键不是“哪种文件系统技术更纯粹”，而是：

- 普通用户很难可靠使用 `FUSE` 作为默认入口
- 但是人类和 agent 仍然需要在同一份本地工作区里协作

所以主线改成：

1. **sync workspace 是主线**
2. **execution contract 单独定义**
3. **FUSE 只保留为 future / advanced mode**

## 产品承诺

Section 主线应该承诺的是：

- 用户得到一个可工作的本地 workspace
- workspace 背后可以连接不同介质和协议
- 文件内容、目录结构、materialization 状态、同步状态、冲突状态是可见和可控的
- 人类和 agent 都在同一份本地 workspace 上工作

Section 主线不承诺的是：

- macOS / Linux / Windows 的原生执行语义完全一致
- 完整 POSIX metadata 在跨平台间严格 round-trip
- 第一版就提供任意挂载点、原生 mount、零差异文件系统体验

## 用户与交互面

| 用户 / 进程 | 主访问方式 | 说明 |
|-------------|------------|------|
| 人类用户 | 本地 workspace 目录 | Finder / Explorer / shell / editor |
| AI Agent | 同一份本地 workspace | 不走专有 API 作为主路径 |
| CLI / GUI / API | 控制面 | 管 workspace、状态、同步、冲突、pin/materialize |
| Runtime | 执行面 | 解释器 / 容器 / WSL / remote runner |

## 产品分层

### Workspace Plane

这是主工作面：

- 本地目录
- 普通文件 / 目录工作流
- agent 与人类共享
- sync / materialize / conflict 都围绕它发生

### Control Plane

这是管理面：

- source / workspace 管理
- status / health / diagnostics
- sync / materialize / pin / repair
- 配置、认证、安装引导

### Execution Plane

这是单独的一层，不应和 workspace 混成一个承诺：

- Python / Node / Java 等显式 runtime
- POSIX shell runtime
- Windows 原生 runtime
- 容器 / WSL / remote runner

Section 统一 workspace，不直接承诺统一宿主机执行语义。

## Workspace Contract

主线 MVP 应先定义清楚：

### 保证的对象

- regular files
- directories
- 内容同步
- 基础状态可见：
  - synced / pending / materialized / conflict

### 暂不保证的对象

- 完整权限 / owner / ACL 跨平台同步
- 完整 `xattr`
- 完整 `symlink / hardlink`
- 设备文件、socket、FIFO 等复杂对象

### 核心产品动作

- attach source
- create workspace
- materialize / pin
- observe local changes
- sync remote changes
- surface conflicts honestly

## 为什么不是 sync engine API 先行

Section 主线是 `sync workspace`，但这不等于第一版就必须绑定：

- macOS File Provider
- Windows CFAPI

第一阶段更合理的是：

- 先把 Section 自己的 workspace contract 做出来
- 用普通本地目录把同步工作流跑通
- 原生 sync engine / shell integration 作为第二阶段增强

否则会过早把产品定义绑定到平台专有模型。

## 为什么不是 FUSE 主线

原因非常现实：

- 普通用户安装和批准复杂度过高
- 跨平台支持成本高
- 即使统一了文件系统入口，也不能统一 Windows 的原生执行语义

因此 `FUSE` 不是主产品入口，而是：

- Linux / advanced mode
- future enhancement
- power user path

## 平台策略

| 能力 | Linux | macOS | Windows | 主线判断 |
|------|-------|-------|---------|----------|
| core/provider/CLI | 支持 | 支持 | 目标支持 | 必须持续 green |
| sync workspace | 主线 | 主线 | 主线 | 产品默认入口 |
| 原生 mount / adapter | advanced | advanced / future | advanced / future | 不是 MVP 前提 |
| execution | runtime 负责 | runtime 负责 | runtime 负责 | 不由 FS 层硬统一 |

## MVP 定义（pivot 后）

新的 MVP 应该是：

1. 能连接一个或多个 source
2. 能把 regular files / directories 同步或 materialize 到本地 workspace
3. 本地修改与远端修改都能被收敛
4. conflict / pending / offline 状态可见
5. agent 与人类都在同一份本地 workspace 工作
6. execution 通过明确 runtime 路线承接，而不是依赖“天然 POSIX 等价”

## Future Tracks

下面这些都保留，但不再作为主线前提：

- Linux `FUSE` / macOS `macFUSE` / Windows `WinFsp`
- macOS File Provider
- Windows Cloud Files API
- SMB 导出 / shared workspace server mode

它们应该建立在稳定的 workspace contract 之上，而不是反过来定义产品。
