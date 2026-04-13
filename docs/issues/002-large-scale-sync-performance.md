# Issue #002: 大体量文件同步性能优化

## Status

`draft`

## Summary

当前 sync 在文件数量大（1000+）或文件体积大（MB~GB 级）时存在显著性能问题：
- 每轮全量扫描本地文件并计算 SHA-256
- 每轮全量 list 远程 bucket，并且在缺少稳定 metadata 时可能回退到远程读对象体
- 所有传输串行执行
- `source sync --watch` 模式仍是固定间隔轮询（默认 2 秒）

本 issue 聚焦于让 Section 在大体量场景下仍能保持流畅的同步体验。

## Problem Analysis

### 场景：10,000 个文件，平均 100KB

| 操作 | 当前行为 | 预估耗时 | 网络开销 |
|------|---------|---------|---------|
| 本地扫描 | 读全部文件 + 算 SHA-256 | ~2-5s（磁盘 I/O 瓶颈） | 无 |
| 远程扫描 | `list` 全部对象 + 必要时读对象体求版本 | ~1-3s+ | ~1-5MB + 额外对象读取 |
| 变更对比 | 内存 HashMap 对比 | ~1ms | 无 |
| 文件传输（无变化时） | 不传输 | 0 | 0 |
| **单轮无变化总开销** | | **~3-8s** | **~1-5MB + 额外回退读** |

在 `source sync --watch` 模式下，默认每 2 秒一轮；无变化场景下，大部分时间都消耗在重复扫描和轮询上。

### 场景：100 个文件，其中 1 个 500MB 文件有变化

| 操作 | 当前行为 | 问题 |
|------|---------|------|
| 检测 500MB 文件变化 | 读 500MB + 算 SHA-256 | 浪费，ETag/mtime 就够了 |
| 传输变化 | 整个文件重传 | 没有分块增量 |
| 并发 | 串行传输 | 其他小文件等着 |

### 根本原因

1. **没有 mtime/size 快速路径** — 无论文件是否变化都全量读
2. **远程版本探测可能读对象体** — 缺少稳定 `etag/last_modified/size` 时，扫描阶段就可能退化成读远程内容
3. **watch 模式是轮询** — 本地无变化时也按间隔重复做全量判断
4. **没有并行传输** — 大文件阻塞小文件
5. **缺少分层传输接口** — 还没有可复用的并发、重试、流式传输挂载点
6. **没有进度反馈** — 大文件传输时用户无感知

## Proposals

### P0: 本地 mtime + size 快速跳过

**目标**：无变化文件零 I/O。

```
每轮 sync:
  对每个本地文件:
    if cached_mtime == current_mtime && cached_size == current_size:
      跳过，直接用缓存的 hash
    else:
      读取文件，算 SHA-256，更新缓存
```

缓存存储在 `.section/mtimes.json` 或 SQLite 表中。

**预期效果**：无变化时本地扫描从 O(文件总大小) 降到 O(文件数量 × stat 调用)，10000 个文件从 ~3s 降到 ~50ms。

**依赖**：无，可独立实现。

### P1: 本地文件监视（inotify / FSEvents / ReadDirectoryChangesW）

**目标**：优化 `source sync --watch` 模式的空转成本。

使用 `notify` crate 跨平台监视本地目录变化：
- 文件创建/修改/删除 → 生成变更事件
- 只对收到事件的文件做 sync
- 无事件时 sync 空操作

```
当前: watch 模式默认每 2s 全量扫描 → 99% 空转
改为: 内核通知变化 → 只处理变化的文件
```

**预期效果**：watch 模式在本地无变化时接近零 CPU/I/O；仍保留一次性的初始同步和降级轮询路径。

**依赖**：P0 的 mtime 缓存可作为监视不可用时的降级方案。

### P2: 远程 metadata diff（正确性基线） + Provider 加速通道

**目标**：在不牺牲正确性的前提下，降低远程检测成本。

先明确一条约束：
- `ListObjectsV2 continuation token` 只能用于单次列表请求的分页，不能作为“自上次同步以来的增量游标”
- 因此，不能把 continuation token 作为跨轮次变更检测方案

#### P2a: metadata manifest diff（所有 provider 的正确基线）

- 每轮执行分页的递归 `list`
- 仅依赖 metadata 构建远程 manifest：`{path, kind, etag?, last_modified?, size?}`
- 将本轮 manifest 与上轮 manifest diff，正确识别新增、修改、删除
- 当 metadata 足够稳定时，不再为“无变化对象”读取对象体
- 对于 metadata 不完整或不稳定的 provider，只对“模糊对象”回退到 `stat` 或内容哈希

这不是“真增量”，但它满足两个关键条件：
- **正确**：能发现全新的远程路径，而不是只检查已知路径
- **更便宜**：把远程内容读取从“扫描阶段默认行为”降到“仅在 metadata 不足时发生”

#### P2b: Provider 加速通道（可选）

在 P2a 正确基线之上，为支持能力的 provider 增加更便宜的变化来源：
- **S3 bucket notification**：对象创建/删除/覆盖事件推送到 Section
- **对象清单/库存文件**：由存储侧周期性导出 manifest，Section 增量消费
- **prefix sharding + 热前缀优先扫描**：当源目录分布有规律时，优先扫描近期活跃前缀

这些方案都只能作为“加速器”，不能替代 P2a 的正确性回退路径。

### P3: 并行传输

**目标**：多文件并发 pull/push，互不阻塞。

基于 Issue #001 的 `Transport` 层做并发执行：
- 小文件并发度可以拉到 8-16
- 大文件与小文件分开调度，避免 head-of-line blocking
- 传输队列优先级：小文件优先，避免一个大文件阻塞整体进度

**预期效果**：100 个文件各 100KB 时，从串行 ~10s 降到并行 ~1s。

**依赖**：Issue #001 的 Transport 层重构。

### P4: 传输进度反馈

**目标**：大文件传输时用户有感知。

分成两层：

#### P4a: 生命周期事件（可先做）

- sync 命令输出当前正在处理的文件名和阶段（queued/running/completed/failed）
- watch 事件流中增加 `syncing` 状态事件（开始/完成/失败）

#### P4b: 字节级进度（依赖流式传输）

- 对 pull/push 改成流式读写后，再输出字节数和百分比
- 对于 CLI，显示 `pulling docs/large-dataset.csv [=====>    ] 45% 12.3MB/27.1MB`

当前整文件 `read`/`write` 实现无法稳定提供百分比，因此百分比进度条不是本 issue 的前置交付，而是 P4b 的后续能力。

**依赖**：P3 的并行传输框架。

### P5: 大文件分块增量传输（远期）

**目标**：只传文件变化的部分。

实现 rolling hash 分块，对比本地和远程的分块签名，只传输差异块。类似 rsync 的 `--delta-transfer`。

**复杂度高，收益取决于场景**：
- 日志文件追加写入 → 收益大
- 二进制文件全量覆盖 → 没有收益
- 需要远程端支持分块读取（S3 Range GET 可用）

**依赖**：P3 并行传输 + 远程分块元数据存储。

## Acceptance Criteria

### P0: mtime 快速跳过
- [ ] mtime/size 缓存持久化到 SQLite
- [ ] sync 时 mtime/size 未变的文件跳过文件读取
- [ ] 缓存命中率指标可观测
- [ ] 10000 个无变化文件的 sync 在 500ms 内完成

### P1: 本地文件监视
- [ ] Linux (inotify) + macOS (FSEvents) 支持
- [ ] `source sync --watch` 模式在本地无变化时不再触发全量文件读取
- [ ] 文件变更在 1s 内触发 sync
- [ ] 监视失败时降级为当前轮询行为（默认 2 秒，可配置）

### P2a: 远程 metadata diff
- [ ] 远程 manifest 持久化到 SQLite
- [ ] 能正确检测远程新增/修改/删除，包括此前未知的新路径
- [ ] 当 provider 提供稳定 `etag` 或 `last_modified+size` 时，无变化对象的扫描阶段对象体读取次数为 0
- [ ] 当 metadata 不足时，仅对模糊对象回退到 `stat` 或内容哈希，不允许对全量对象无差别读取

### P2b: Provider 加速通道
- [ ] 至少为一种 S3 兼容 provider 提供可选加速通道（事件或库存清单）
- [ ] 加速通道关闭时，系统仍可退回 P2a 并保持正确
- [ ] 加速通道开启时，无变化场景下远程请求量显著低于完整递归 list

### P3: 并行传输
- [ ] 多文件 pull/push 并发执行
- [ ] 并发度可配置（默认 8）
- [ ] 单文件大文件不阻塞其他文件

### P4a: 生命周期反馈
- [ ] CLI 模式显示当前传输文件和阶段
- [ ] watch 事件流包含 syncing/ready/error 等状态变化

### P4b: 字节级进度
- [ ] 在流式 transport 落地后，CLI 可显示字节数与百分比
- [ ] 并发传输时，每个活跃文件都有稳定的进度来源

### P5: 大文件分块（远期）
- [ ] 原型验证 S3 Range GET + rolling hash 可行性

## Priority

P0 → P2a → P3 → P1 → P4a → P2b → P4b，P5 为远期探索。

建议先做 P0 + P2a，先把“本地/远程扫描的 correctness 和大头 I/O”降下来；再做 P3，解决传输阻塞；随后补 P1 和反馈能力。

## References

- `crates/sectiond/src/sync.rs` — 当前 sync 实现
- Issue #001 — Sync 架构重构（P3 并行传输依赖此重构）
- `docs/SYNC_MODEL.md` — 同步模型契约
- OpenDAL Layers: https://docs.rs/opendal/latest/opendal/layers/index.html
