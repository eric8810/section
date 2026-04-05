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
2. **execution 不是当前项目范围**

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

这里还必须承担一件事：

- 提供普通文件系统本体无法表达的 sync / version / compare / resolve 能力

### Execution Non-Goal

当前项目只需要把边界说清楚：

- Section 不承诺统一宿主机执行语义
- 当前项目不定义 scripts / agents 的执行方案
- 当前项目只保证 source/path sync 与冲突处理

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
- resolve conflicts explicitly

### 冲突解决方案

MVP 不做自动 merge，也不做版本分叉模型。

这里的 `conflict` 只表示一件事：

- 本地上传基于旧的 remote 状态，Section 拒绝做 blind overwrite

发生冲突时：

- path 状态进入 `conflict`
- 该 path 的自动同步暂停
- 本地当前版本保留不动
- 当前 remote 版本不会被自动覆盖

用户只允许 2 种解决动作：

- `use-local`
- `use-remote`

如果用户想手工 merge，也是在本地编辑完成后，最终再执行一次 `use-local`。

只有显式 resolve 后，这个 path 才恢复正常同步。

### 应用如何得知状态与版本

结论很直接：

- 普通应用只会看到“文件已经在本地”
- 普通 POSIX 文件本体不能可靠表达 sync state / remote version / resolve action
- 这些信息必须通过 Section control plane 获取

所以当前产品应明确区分两层：

- data plane：本地文件树，供 editor / shell / agent 正常读写
- control plane：供 Section-aware CLI / GUI / API 查询状态、版本、对比和 resolve

最小 control-plane 能力应是：

- `path inspect`
  - 返回 `ready / syncing / conflict / error`
  - 返回 detail fields
  - 返回 `base_remote_version`
  - 返回 `current_remote_version`
- `path compare`
  - 告诉调用方本地内容是否基于当前 remote
  - 暴露 local / remote 的可对比引用或快照信息
- `path resolve --strategy use-local|use-remote`
  - 显式完成冲突取舍

也就是说：

- 应用本身不会“自动知道”
- Section-aware 客户端必须主动调用 control plane

### Agent 如何发现这是 Section 目录

如果本地目录里完全没有入口，agent 就无从发现。

所以每个 bound local root 都应该放一个极轻量的本地 marker：

- `.section/root.json`

它不是同步状态数据库，只承担 discovery 作用，至少包含：

- `source_id`
- `local_root`
- `sectiond` control-plane endpoint

agent 的发现流程应是：

1. 从当前工作路径开始向上找
2. 找到 `.section/root.json`
3. 确认当前路径属于某个 Section-bound local root
4. 再调用 `path inspect / compare / resolve`

这样区分很清楚：

- 本地文件树负责日常读写
- root marker 负责 discovery
- control plane 负责 sync truth

但对 agent 的常用入口，不该要求它手动解析 marker 再自己拼 source/path。

更合理的 agent UX 应该是：

- `section path inspect ./some/local/file --json`
- `section path compare ./some/local/file --json`
- `section path resolve ./some/local/file --strategy use-local`

也就是说：

- 命令直接接受本地路径
- CLI/GUI/API 在内部完成：
  - 向上查找 `.section/root.json`
  - 识别 source 与 local root
  - 把本地路径映射成真实的 source/path
  - 返回 sync truth 或执行 resolve

这样 agent 的正常使用流程才不会太笨重。

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
6. 当前项目不处理 execution 方案，只明确它不在承诺范围内


它们应该建立在稳定的 source/path sync contract 之上，而不是反过来定义产品。
