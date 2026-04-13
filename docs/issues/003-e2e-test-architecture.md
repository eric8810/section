# Issue #003: E2E 测试架构设计

## Status

`draft`

## Summary

当前测试只覆盖了 CLI 的基本功能（source 管理、文件读写拷贝、缓存刷新），用的是本地 `fs` provider。
同步核心路径（sync、conflict、双向传输）只有两个传统的 `#[test]`，没有 BDD 覆盖。
没有任何测试针对真实远程存储（S3/RustFS）、大体量文件、或性能边界场景。

随着 Issue #001（架构重构）和 Issue #002（性能优化）推进，需要一套更完整的 E2E 测试架构来保障正确性和可回归。

## Current State

### 现有测试结构

```
tests/features/                          # 当前 Cucumber feature 文件 (6个, 33个场景)
├── source_manage.feature                # source add/remove/list
├── file_read.feature                    # ls / cat
├── file_write.feature                   # write / rm
├── file_copy.feature                    # cp 跨源/本地
├── file_exec.feature                    # exec 远端执行
└── cache_refresh.feature                # refresh 缓存

crates/section-cli/tests/
├── bdd.rs                               # Cucumber World + main()
├── path_control_plane.rs                # #[test] bind/inspect/list
├── sync_control_plane.rs                # #[test] sync/compare/watch/resolve
└── steps/
    ├── common.rs                        # Given: 临时目录、测试文件、source 配置
    ├── file_steps.rs                    # When: pipe 命令
    └── source_steps.rs                  # When/Then: 执行 + 断言
```

### 覆盖现状

| 能力 | BDD 覆盖 | Integration 覆盖 | 用到的 provider |
|------|---------|-----------------|----------------|
| source 管理 | ✅ 6 个场景 | — | fs |
| 文件读 | ✅ 11 个场景 | — | fs |
| 文件写/删 | ✅ 4 个场景 | — | fs |
| 文件拷贝 | ✅ 7 个场景 | — | fs |
| 文件执行 | ✅ 4 个场景 | — | fs |
| 缓存刷新 | ✅ 1 个场景 | — | fs |
| bind/inspect | — | ✅ 1 个测试 | fs |
| sync/compare/resolve | — | ✅ 1 个测试 | fs |
| **S3/远程存储** | **❌** | **❌** | **无** |
| **双向同步** | **❌** | **基本** | **fs** |
| **大体量文件** | **❌** | **❌** | **无** |
| **并发同步** | **❌** | **❌** | **无** |
| **性能边界** | **❌** | **❌** | **无** |

### 核心问题

1. **只有 fs provider** — 所有测试用本地文件系统，没有验证 S3/WebDAV 真实通信
2. **同步测试薄弱** — 双向同步、冲突、远程变更检测只有最基本覆盖
3. **没有大体量测试** — 无法验证 1000+ 文件、大文件、并行传输的正确性
4. **无法测试性能回归** — 没有稳定的 perf regression 套件，优化后无法量化对比
5. **Issue #001 重构后需要分层测试** — Change Detector / Transport 需要独立可测

## Proposal

### 测试金字塔

```
                    ┌─────────────┐
                    │  Smoke Test │   ← 真实 S3，几个文件，验证基本链路
                    │   (CI 快)   │
                   ┌┴─────────────┴┐
                   │ Integration    │  ← fs mock + 真实 sync 逻辑
                   │ (每个 PR 跑)   │
                  ┌┴───────────────┴┐
                  │  BDD Feature     │ ← 用户场景覆盖
                  │ (每个 PR 跑)     │
                 ┌┴─────────────────┴┐
                 │  Performance       │ ← 大体量 + benchmark
                 │  (手动/nightly)    │
                 └───────────────────┘
```

### 1. 测试分层：为 Issue #001 服务

架构重构后，每层需要独立的测试：

#### Layer Test: Change Detector

```
输入: 模拟的本地文件树 + 远程对象列表 + 上次状态
输出: Vec<PathSyncInput>
验证: collector 产出的 `previous/local/remote` 事实与预期一致
不需要: 真实 S3 连接，文件传输
```

```rust
#[test]
fn collect_local_new_file() {
    let collector = SnapshotCollector::new(temp_dir);
    // 写入一个新文件
    fs::write(temp_dir.join("new.txt"), "hello").unwrap();
    let local = collector.collect_local(&temp_dir, &HashMap::new()).unwrap();
    let entry = local.get("new.txt").unwrap();
    assert_eq!(entry.kind, EntryKind::File);
    assert!(entry.version.is_some());
}

#[test]
fn build_inputs_preserves_previous_state() {
    let collector = SnapshotCollector::new(temp_dir);
    // 首次扫描建立缓存
    let previous = seed_previous_state("docs/readme.txt");
    let local = collector.collect_local(&temp_dir, &previous).unwrap();
    let inputs = collector.build_inputs(previous, local, HashMap::new());
    assert!(inputs.iter().any(|input| input.path == "docs/readme.txt"));
}
```

#### Layer Test: Transport

```
输入: SyncPlan (pull/push 文件列表)
输出: 传输成功/失败
验证: 文件内容正确，重试生效，并发可控
需要: 真实或 mock 的 Operator
```

```rust
#[test]
fn transport_parallel_pull() {
    let op = test_s3_operator(); // 或 mock operator
    let transport = ParallelTransport::new(op, concurrency: 4);
    let plan = SyncPlan { pull: vec!["a.txt", "b.txt", "c.txt"], .. };
    let result = transport.execute(plan).unwrap();
    assert_eq!(result.pulled, 3);
    // 验证本地文件内容
}
```

#### Layer Test: Sync Coordinator

```
输入: PathSyncInput
输出: SyncPlan + 冲突决策
验证: 状态机逻辑正确（各种变更组合下的决策）
不需要: 文件 I/O
```

```rust
#[test]
fn coordinator_detects_conflict() {
    let input = PathSyncInput {
        path: "readme.txt".into(),
        previous: Some(seed_previous_state("readme.txt")),
        local: Some(file_entry("local-v2")),
        remote: Some(file_entry("remote-v2")),
    };
    let plan = SyncCoordinator::plan_one(input).unwrap();
    assert!(plan.ops.iter().any(|op| matches!(op, PlannedOp::MarkConflict { .. })));
}
```

### 2. Provider Matrix 测试

为每种 provider 建立测试 fixture：

| Provider | 测试环境 | 用途 |
|----------|---------|------|
| `fs` | 本地临时目录 | 基础功能验证（当前已有） |
| `s3` | RustFS Docker 容器 | 真实 S3 协议验证 |
| `webdav` | 可选，暂不阻塞 | |

#### S3 测试 Fixture 设计

```rust
struct S3Fixture {
    docker: Docker,         // 管理 RustFS 容器生命周期
    endpoint: String,       // http://localhost:XXXX
    access_key: String,
    secret_key: String,
    bucket: String,
    operator: Operator,     // 预配置的 OpenDAL Operator
}

impl S3Fixture {
    fn new() -> Self {
        // 启动 RustFS Docker 容器
        // 创建 bucket
        // 返回 fixture
    }

    fn put_object(&self, key: &str, data: &[u8]) { ... }
    fn get_object(&self, key: &str) -> Vec<u8> { ... }
    fn list_objects(&self) -> Vec<String> { ... }
}
```

CI 环境中 RustFS 容器通过 `docker compose` 或 `testcontainers` 管理。

### 3. BDD Runner 拆分

当前 `bdd.rs` 会直接运行整个 `tests/features` 目录，因此不能把需要 Docker/S3 的 feature 混进现有目录后继续在 PR 上执行。

目标方案：

```
tests/
├── features-fs/                         # PR 必跑
└── features-s3/                         # Nightly / 手动触发

crates/section-cli/tests/
├── bdd_fs.rs                            # 只跑 tests/features-fs
└── bdd_s3.rs                            # 只跑 tests/features-s3
```

约束：
- `bdd_fs.rs` 保持无外部依赖，PR 必跑
- `bdd_s3.rs` 启动或连接 RustFS fixture，Nightly/手动跑
- 不依赖“把 S3 feature 放进同一目录后再靠约定跳过”的隐式行为
- 若环境变量 `SKIP_S3_TESTS=1`，`bdd_s3.rs` 直接退出并标记 skipped
### 4. Sync E2E 场景（BDD 补充）

需要新增的 feature 文件：

#### `sync_basic.feature` — 基本同步场景

```gherkin
功能: 基本同步
  场景: 远程新文件同步到本地
  场景: 本地新文件同步到远程
  场景: 远程修改文件同步到本地
  场景: 本地修改文件同步到远程
  场景: 远程删除文件同步到本地（本地文件被删除）
  场景: 本地删除文件同步到远程（远程文件被删除）
  场景: 双向无变化时空操作
```

#### `sync_conflict.feature` — 冲突场景

```gherkin
功能: 冲突检测与解决
  场景: 双方同时修改同一文件 → conflict
  场景: conflict 状态下 sync 不覆盖
  场景: resolve --strategy use-local → 本地覆盖远程
  场景: resolve --strategy use-remote → 远程覆盖本地
  场景: 本地删除 + 远程修改 → conflict
  场景: 远程删除 + 本地修改 → conflict
```

#### `sync_s3.feature` — S3 真实通信

```gherkin
功能: S3 同步
  场景: bind S3 source 并 sync
  场景: 本地文件 push 到 S3
  场景: S3 新文件 pull 到本地
  场景: S3 大文件同步（10MB+）
```

#### `sync_watch.feature` — Watch 事件

```gherkin
功能: Watch 事件
  场景: sync 后 watch 捕获 synced_from_remote 事件
  场景: sync 后 watch 捕获 synced_to_remote 事件
  场景: 冲突时 watch 捕获 conflict_detected 事件
```

`sync_basic.feature`、`sync_conflict.feature`、`sync_watch.feature` 放到 `tests/features-fs/`。
`sync_s3.feature` 放到 `tests/features-s3/`，由独立 runner 执行。

### 5. 大体量 Performance / Regression 测试

为 Issue #002 的优化提供回归保障：

```rust
// crates/section-cli/tests/perf_sync.rs

#[test]
fn bench_scan_10k_files() {
    let dir = generate_file_tree(10_000, avg_size: 1024);
    let start = Instant::now();
    let changes = detector.detect_local_changes(&dir).unwrap();
    let elapsed = start.elapsed();
    assert!(elapsed < Duration::from_millis(500), "scan took {:?}", elapsed);
    assert!(changes.added.is_empty()); // 无变化
}

#[test]
fn bench_sync_with_100_changes() {
    let fixture = S3Fixture::new();
    // 准备: 10000 个文件，其中 100 个有变化
    let start = Instant::now();
    let result = sync_source(&runtime, &store, "bench", &local_root).unwrap();
    let elapsed = start.elapsed();
    assert_eq!(result.pulled + result.pushed, 100);
    assert!(elapsed < Duration::from_secs(5));
}
```

测试数据生成器：

```rust
mod fixture_gen {
    /// 在指定目录生成 N 个文件，平均每个 size 字节
    fn generate_file_tree(dir: &Path, count: usize, avg_size: usize) { ... }

    /// 生成嵌套目录结构（depth 层，每层 spread 个子目录）
    fn generate_nested_tree(dir: &Path, depth: usize, spread: usize) { ... }

    /// 在已有树中随机修改 ratio 比例的文件
    fn modify_random_files(dir: &Path, ratio: f64) -> Vec<PathBuf> { ... }
}
```

建议把这类测试定义为 `perf regression test` 而不是通用 benchmark：
- 默认 `#[ignore]`
- Nightly 或手动触发
- 输出 JSON 结果，便于和基线比较
- 阈值以“相对基线”或宽阈值为主，避免 CI 抖动

### 6. CI 集成

```
PR 提交时:
  cargo test                          # unit tests
  cargo test --test bdd_fs            # BDD (fs provider)
  cargo test --test path_control_plane
  cargo test --test sync_control_plane

Nightly / 手动触发:
  docker compose up rustfs            # 启动 RustFS
  cargo test --test bdd_s3            # S3 BDD
  cargo test --test sync_s3           # S3 控制面/冒烟测试
  cargo test --test perf_sync -- --ignored --nocapture
```

## 目录结构（目标）

```
tests/
├── features-fs/                       # PR 必跑的 BDD
│   ├── source_manage.feature
│   ├── file_read.feature
│   ├── file_write.feature
│   ├── file_copy.feature
│   ├── file_exec.feature
│   ├── cache_refresh.feature
│   ├── sync_basic.feature             # 新增
│   ├── sync_conflict.feature          # 新增
│   └── sync_watch.feature             # 新增
│
├── features-s3/                       # 真实 provider BDD
│   └── sync_s3.feature                # 新增
│
├── fixtures/                          # 新增: 测试 fixture 和数据生成
│   ├── mod.rs
│   ├── s3.rs                          # RustFS Docker fixture
│   ├── fs.rs                          # 本地文件系统 fixture
│   ├── data_gen.rs                    # 大体量测试数据生成器
│   └── assertions.rs                  # 同步相关的自定义断言
│
crates/sectiond/tests/                 # 新增: 分层单元测试
├── change_detector_test.rs
├── sync_coordinator_test.rs
└── transport_test.rs
│
crates/section-cli/tests/
├── bdd_fs.rs                          # 新增/由现有 bdd.rs 演进
├── bdd_s3.rs                          # 新增
├── path_control_plane.rs              # 现有
├── sync_control_plane.rs              # 现有
├── sync_s3.rs                         # 新增: S3 E2E
├── perf_sync.rs                       # 新增: perf regression tests
└── steps/
    ├── common.rs                      # 现有，扩展 S3 fixture 支持
    ├── file_steps.rs                  # 现有
    ├── source_steps.rs                # 现有
    └── sync_steps.rs                  # 新增: sync 相关步骤
```

## Acceptance Criteria

### Phase 1: 同步 BDD 覆盖（Issue #001 前置）
- [ ] `tests/features-fs/sync_basic.feature` — 7 个基本同步场景通过
- [ ] `tests/features-fs/sync_conflict.feature` — 6 个冲突场景通过
- [ ] `tests/features-fs/sync_watch.feature` — 3 个事件场景通过
- [ ] `bdd_fs.rs` 只执行 `features-fs/`
- [ ] 新增 `sync_steps.rs` 步骤定义

### Phase 2: S3 真实测试
- [ ] `S3Fixture` — RustFS Docker 容器自动管理（启动/停止/清理）
- [ ] `tests/features-s3/sync_s3.feature` — S3 基本同步场景通过
- [ ] `bdd_s3.rs` 只执行 `features-s3/`
- [ ] S3 测试可通过环境变量跳过（`SKIP_S3_TESTS=1`），CI 无 Docker 时不阻塞

### Phase 3: 分层测试（配合 Issue #001 重构）
- [ ] `change_detector_test.rs` — mtime 缓存、增量检测独立测试
- [ ] `sync_coordinator_test.rs` — 状态机逻辑独立测试
- [ ] `transport_test.rs` — 并行传输、重试独立测试
- [ ] 每层 mock 其下层依赖，可独立运行

### Phase 4: Performance 回归（配合 Issue #002 优化）
- [ ] `data_gen.rs` — 可生成 100~100000 文件的测试树
- [ ] `perf_sync.rs` — 至少 3 个 perf regression tests（扫描/同步/传输）
- [ ] perf regression 结果可输出为 JSON 用于对比

## Dependencies

- Issue #001（架构重构）— Phase 3 依赖重构完成
- Issue #002（性能优化）— Phase 4 依赖优化实现
- Docker — Phase 2 需要 RustFS 容器
- `testcontainers` crate（可选）— 自动管理 Docker 容器

## References

- `tests/features/` — 现有 6 个 BDD feature 文件
- `crates/section-cli/tests/bdd.rs` — Cucumber 测试入口
- `crates/section-cli/tests/steps/` — 步骤定义
- `docs/issues/001-sync-architecture-refactor.md`
- `docs/issues/002-large-scale-sync-performance.md`
