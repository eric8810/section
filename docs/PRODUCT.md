# Section Product Model

## 一句话定义

Section 是一个**跨平台 source/path sync 协作层**：

- 保持 `source/path` 作为主 mental model
- 把任意后端介质同步到本地目录
- 让人类、agent、shell、editor 对着同一份本地 path 工作
- 不把“统一宿主机 POSIX 执行语义”当成主产品承诺

## 核心产品判断

这次 pivot 的关键不是“哪种文件系统技术更纯粹”，而是：

- 普通用户很难可靠使用 `FUSE` 作为默认入口
- 但是人类和 agent 仍然需要在同一份本地目录里协作
- 新路线不该为了 sync 再发明一个额外的顶层对象

所以主线改成：

1. **source/path + sync state 是主线**
2. **execution contract 单独定义**

## 产品承诺

Section 主线应该承诺的是：

- 用户面对的仍然是 `source/path`
- 某个 source 可以绑定一个本地目录
- 文件内容、目录结构、同步状态、冲突状态是可见和可控的
- 人类和 agent 都在同一份本地 path 上工作
- 对外状态保持简单：`ready / syncing / conflict / error`

Section 主线不承诺的是：

- macOS / Linux / Windows 的原生执行语义完全一致
- 完整 POSIX metadata 在跨平台间严格 round-trip
- 第一版就提供任意挂载点、原生 mount、零差异文件系统体验

## 用户与交互面

| 用户 / 进程 | 主访问方式 | 说明 |
|-------------|------------|------|
| 人类用户 | 本地 path | Finder / Explorer / shell / editor |
| AI Agent | 同一份本地 path | 不走专有 API 作为主路径 |
| CLI / GUI / API | 控制面 | 管 source/path、状态、同步、冲突、pin/pull |
| Runtime | 执行面 | 解释器 / 容器 / WSL / remote runner |

## 产品分层

### Source/Path Plane

这是主工作面：

- `source/path`
- source 绑定到本地目录
- 普通文件 / 目录工作流
- agent 与人类共享
- sync / local-ready / conflict 都围绕 source 与 path 状态发生

### Control Plane

这是管理面：

- source 管理
- source 绑定本地目录
- status / health / diagnostics
- sync / pull / pin / repair
- 配置、认证、安装引导

### Execution Plane

这是单独的一层，不应和 source/path sync 混成一个承诺：

- Python / Node / Java 等显式 runtime
- POSIX shell runtime
- Windows 原生 runtime
- 容器 / WSL / remote runner

Section 统一的是 source/path sync contract，不直接承诺统一宿主机执行语义。

## Source/Path Sync Contract

主线 MVP 应先定义清楚：

### 保证的对象

- regular files
- directories
- 内容同步
- 基础状态可见：
  - ready / syncing / conflict / error

### 对外与对内的边界

对外主状态只保留：

- `ready`
- `syncing`
- `conflict`
- `error`

下面这些不应成为用户主心智，只作为详情字段或内部实现：

- `local_present`
- `dirty_local`
- `dirty_remote`
- `pinned`
- `stale`
- error reason

### 暂不保证的对象

- 完整权限 / owner / ACL 跨平台同步
- 完整 `xattr`
- 完整 `symlink / hardlink`
- 设备文件、socket、FIFO 等复杂对象

### 核心产品动作

- attach source
- bind local root
- pull / pin
- observe local changes
- sync remote changes
- surface conflicts honestly

## 平台策略

| 能力 | Linux | macOS | Windows | 主线判断 |
|------|-------|-------|---------|----------|
| core/provider/CLI | 支持 | 支持 | 目标支持 | 必须持续 green |
| source/path sync | 主线 | 主线 | 主线 | 产品默认入口 |
| execution | runtime 负责 | runtime 负责 | runtime 负责 | 不由 FS 层硬统一 |

## MVP 定义（pivot 后）

新的 MVP 应该是：

1. 能连接一个或多个 source
2. 能为 source 绑定本地目录，并把 regular files / directories 可靠同步到本地
3. 本地修改与远端修改都能被收敛
4. source/path 级别的 `ready / syncing / conflict / error` 状态可见
5. agent 与人类都在同一份本地 path 工作
6. execution 通过明确 runtime 路线承接，而不是依赖“天然 POSIX 等价”


它们应该建立在稳定的 source/path sync contract 之上，而不是反过来定义产品。
