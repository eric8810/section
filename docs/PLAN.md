# Section Route Map

## 方向结论

Section 现在不再按“多后端 CLI + FUSE 功能集合”来规划，而是按下面这个目标统一：

> agent 和人类应该在同一个 FS 介质上协作，路径语义一致，shell / editor / script 的心理模型一致。

这意味着：

- **FS 是主交互面**
- **CLI / API 是 control plane**
- **本地长期驻留核心 (`sectiond`) 是下一阶段的真正中心**

详细设计见 [ARCHITECTURE.md](./ARCHITECTURE.md)。

## 当前已完成的基础工作

这些工作不废，属于新路线的 groundwork：

- 多 backend 接入：
  - fs
  - S3
  - WebDAV
- provider store / 凭证加密
- 基础 CLI：
  - source add/remove/list
  - ls/cp/cat/rm/write/exec
  - status / refresh
- 双平台 non-FUSE CI
- Linux FUSE happy path 已验证并文档化

对应的已关闭 issues：

- `#1` repo / docs / support matrix truth alignment
- `#2` cache TTL 语义
- `#3` metadata/content cache 接入 section-fuse
- `#4` refresh / invalidation 路径
- `#5` Linux FUSE mount happy path
- `#6` S3 real backend validation
- `#7` WebDAV real backend validation
- `#8` CLI usability gap
- `#9` router / permission / provider-store coverage
- `#10` platform-aware mount/unmount behavior
- `#12` dual-platform non-FUSE CI

当前唯一延续到新路线中的旧编号 issue：

- `#11` macOS mount adapter shared-workspace validation（当前仍受 macFUSE 依赖阻塞）

## 新路线图

### Phase 1: 明确 sectiond 边界

目标：

- 定义 `sectiond` 与 mount adapter / CLI client 的清晰边界
- 把“共享协作语义”从当前散落的实现中抽出来

产出：

- `sectiond` 职责边界
- control plane / data plane 接口契约
- 状态与缓存归属关系
- 生命周期与健康模型

### Phase 2: 把共享语义真正沉到 sectiond

目标：

- source registry
- routing
- cache
- refresh
- permissions
- diagnostics

都由 `sectiond` 持有，而不是让 CLI / FUSE 各自拥有一套

产出：

- `sectiond` 成为唯一真实本地状态机
- CLI 与 mount adapter 都改为消费它

### Phase 3: Linux 作为正式 data-plane 参考实现

目标：

- Linux FUSE adapter 全量走 `sectiond`
- 保住当前已经验证过的 shell / mount / refresh / write-through 语义

产出：

- Linux 成为“共享 FS 协作模式”的正式参考平台

### Phase 4: 明确执行 / 脚本工作流

目标：

- 让 bash / python / node / editor / agent 都面向同一棵挂载树工作
- 明确 `section exec` 的定位只是辅助，不再当成主路径

产出：

- 统一的执行与 scripting 语义
- 对 mounted workspace 的正式支持边界

### Phase 5: macOS mount adapter

目标：

- 在不改变上层协作模型的前提下，让 macOS 也能进入同一类 FS 心智模型

短期：

- macFUSE adapter

长期：

- 如果 macFUSE 的安装摩擦不可接受，再评估 native adapter

产出：

- macOS 上的真实 mount 验证
- 清晰的 installer / support matrix / fallback 策略

## 新 issue map

下面这些 issue 表示新路线下的主工作流：

| 阶段 | 主题 | GitHub issue |
|------|------|--------------|
| Phase 1 | 定义 `sectiond` 边界与 FS-first 协作契约 | `#13` |
| Phase 2 | 把 routing / cache / refresh / permissions / health 沉到 `sectiond` | `#14` |
| Phase 2 | 把 CLI 重构成 control-plane client | `#15` |
| Phase 3 | 让 Linux mount adapter 通过 `sectiond` 提供正式 data plane | `#16` |
| Phase 4 | 定义共享 mounted workspace 上的 execute / scripting 模式 | `#17` |
| Phase 5 | macOS adapter 的 prerequisite / preflight / installer 策略 | `#18` |
| Phase 5 | macOS mount adapter 的 shared-workspace 真验证 | `#11` |

## 建议执行顺序

建议不要并行乱推，而是按下面顺序推进：

1. `#13` 先把边界和契约定死
2. `#14` 把共享语义沉到 `sectiond`
3. `#15` 让 CLI 明确退到 control plane
4. `#16` 让 Linux data plane 通过 `sectiond` 重新站稳
5. `#17` 在 mounted workspace 上定义真正的脚本 / execute 模式
6. `#18` 产品化 macOS prerequisite / preflight / installer 路径
7. `#11` 在真实 macOS host 上完成 shared-workspace 验证

## 当前 repo truth

截至当前：

- Linux 的真实挂载链路已经证明可行
- macOS 的 non-FUSE 路径是 green 的
- 但 repo 结构本身仍然是 pre-sectiond

所以“下一步”不是继续补零散命令，而是完成一次**架构重心迁移**：

- 从“CLI / FUSE 各自持有逻辑”
- 到“sectiond 统一持有语义，CLI / adapter 只做接口层”

## 不再建议的路线

以下路线现在明确不推荐作为主线：

1. 把 CLI/API-only 当成最终产品形态
2. 在 macOS 还没统一协作心智前就宣称双平台等价
3. 为了回避 macFUSE 安装摩擦，直接把文件系统协作目标降级成“只是多入口访问”
