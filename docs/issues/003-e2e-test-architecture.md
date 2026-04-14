# Issue #003: 测试框架组织

## Status

`draft`

## System Definition

Section 是一个共享工作上下文系统。

它的目标是让目标存储中的一份文件上下文，能够被人类和 agent 在不同终端、不同 OS、不同机器上接入、共享、继续协作。

## Testing Mission

测试体系的目标是：

**验证 Section 能否在声明支持的环境和规模范围内，把目标存储中的工作上下文，以可信、可连续、可移植、且足够及时高效的方式，交付给人类与 agent 共同使用。**

## First-Class Goals

### 1. 可接入性

- 新参与者可以从干净环境接入共享上下文
- 可以完成 source 配置、本地绑定和初始同步
- 接入后的本地工作投影可直接用于工作

### 2. 上下文一致性

- 不同参与者接入的是同一份工作上下文
- 路径、内容、目录结构和公开状态保持一致

### 3. 协作连续性

- 一个参与者的工作结果可被另一参与者继续接手
- 人类与 agent 可以多轮交接而不丢失上下文

### 4. 并发安全性

- 并发修改不会静默破坏共享上下文
- 冲突会被显式暴露
- 冲突后的状态可解释、可解决

### 5. 跨环境可移植性

- 上述能力在声明支持的环境矩阵内成立
- 包括 provider、终端、OS 和参与者类型差异

### 6. 协作时效性与效率

- 接入延迟可接受
- 变更传播延迟可接受
- 冲突显现延迟可接受
- 无变化时空闲成本可接受
- 在不同文件量、网络条件和资源条件下仍具备可用的时间和资源开销

## Minimal Test Dimensions

后续测试设计只控制 3 个维度：

### 1. 协作模式

- 接入
- 交接
- 并发

### 2. 环境矩阵

- OS
- terminal / 调用入口
- provider

### 3. 性能条件

- 文件量
- 网络条件
- 资源条件

资源条件至少包括：

- CPU
- 内存
- 磁盘 I/O

后续所有测试场景都应表达为：

`<协作模式, 环境矩阵, 性能条件>`

## Core Metrics

后续测试体系只保留 4 个核心指标：

### 1. 可工作成功率

- 是否成功进入可继续工作的上下文状态
- 覆盖接入和交接场景

### 2. 语义错误率

- 系统公开状态与真实上下文不一致的比例
- 包括假 `ready`、假一致、控制面语义错误

### 3. 安全违例率

- 并发场景下静默覆盖、漏报冲突等不可接受错误的比例

### 4. 时效达标率

- 接入、传播、空转和典型同步是否落在可接受预算内

后续所有测试结果都应最终归约到这 4 个指标上。

## Framework Organization

测试框架组织为 4 层。

### 1. 共享辅助层

目录：

- `crates/section-cli/tests/support/mod.rs`
- `crates/section-cli/tests/support/fixture.rs`
- `crates/section-cli/tests/support/actor.rs`
- `crates/section-cli/tests/support/check.rs`

职责：

- 创建远端根目录
- 创建多个本地工作目录
- 创建各自的 `config.toml`
- 执行 `section` 命令
- 直接修改本地文件
- 检查本地目录、远端目录、控制面结果
- 记录简单耗时

### 2. 核心场景层

目录：

- `crates/section-cli/tests/attach_flow.rs`
- `crates/section-cli/tests/handoff_flow.rs`
- `crates/section-cli/tests/concurrent_flow.rs`

职责：

- `attach`
- `handoff`
- `concurrent`

第一阶段只做 `fs` provider。

### 3. 环境批量层

- 不为不同 provider、OS 或入口分别设计独立测试目标
- 固定少量核心场景
- 通过不同环境组合批量执行这些场景

环境组合至少包括：

- provider
- OS
- 入口

### 4. 性能验证层

- 使用同一批核心场景
- 只改变性能条件

性能条件至少包括：

- 文件量
- 网络条件
- 资源条件

资源条件至少包括：

- CPU
- 内存
- 磁盘 I/O

## Mainline Validation

- `main` 表示当前可信基线
- 工作中的改动不应直接等同于可信基线
- 每次准备进入 `main` 的改动，都需要先声明其影响的协作模式、环境矩阵和性能条件
- 再执行与该影响面对应的验证
- 验证通过后，该改动才可进入 `main`

## Minimal Rollout

1. 先完成 `support/`
2. 先跑通 `attach`
3. 再跑通 `handoff`
4. 再跑通 `concurrent`
5. 之后复用同一套结构接入其他 provider、OS、入口和性能条件

## Concrete Technical Plan

### Shared Support

在 `crates/section-cli/tests/support/` 下建立共享辅助代码：

- `mod.rs`
- `fixture.rs`
- `actor.rs`
- `check.rs`

### `fixture.rs`

负责：

- 创建远端根目录
- 创建多个本地工作目录
- 创建各自的 `config.toml`
- 提供 `fs` provider 的测试环境
- 为后续 `s3-compatible` provider 预留同样入口

### `actor.rs`

一个参与者对应一个 `Actor`。

先只封装两类操作：

- 执行真实 `section` 命令
- 直接修改本地文件

第一阶段最小能力：

- `add_source`
- `bind`
- `sync`
- `compare`
- `resolve`
- `watch_once`
- `write_local`

### `check.rs`

负责：

- 检查本地工作目录
- 检查远端真实目录
- 检查控制面结果
- 记录耗时

第一阶段输出 4 类结果：

- 是否进入可工作状态
- 是否存在语义错误
- 是否存在安全违例
- 是否满足时间预算

## Concrete Test Cases

第一阶段新增 3 个测试文件：

- `crates/section-cli/tests/attach_flow.rs`
- `crates/section-cli/tests/handoff_flow.rs`
- `crates/section-cli/tests/concurrent_flow.rs`

### `attach_flow.rs`

- 远端先准备一份共享上下文
- 参与者 A 从空环境接入
- 检查本地工作目录是否可工作
- 检查控制面是否与真实结果一致
- 记录第一次接入耗时

### `handoff_flow.rs`

- 参与者 A 接入并修改
- A 同步后，参与者 B 再同步并继续工作
- B 同步后，A 再同步并看到 B 的结果
- 检查交接是否成立

### `concurrent_flow.rs`

- A 和 B 基于同一基线接入
- A 和 B 并发修改同一路径
- 按既定顺序同步
- 检查是否进入 `conflict`
- 检查远端没有发生静默覆盖

## Integration With Existing Tests

- 先从现有 `sync_control_plane.rs` 中抽取公共逻辑
- 优先抽取：
  - `write_config`
  - `run_section`
  - 公共临时目录初始化
- 现有测试先保留
- 新测试先独立落地

## Environment Expansion

- 核心场景固定为 `attach`、`handoff`、`concurrent`
- 环境变化通过替换 `fixture` 和运行环境完成
- 第一阶段只做 `fs`
- 第二阶段接入 `s3-compatible`
- 不为不同 provider、OS、入口重复设计新场景

## Performance Execution

第一阶段只测：

- 第一次接入耗时
- 一次小改动同步耗时
- 冲突显现耗时
- 无变化时一次空转成本

第一阶段通过 `fixture` 构造不同文件量。

网络条件和资源条件不在第一阶段测试代码中模拟，而是在后续环境验证中控制。

## Acceptance Criteria

- [ ] 文档定义测试总目标
- [ ] 文档定义测试框架的层次组织
- [ ] 文档定义共享辅助层的目录和职责
- [ ] 文档定义核心场景层
- [ ] 文档定义环境批量层
- [ ] 文档定义性能验证层
- [ ] 文档定义主线验证策略
