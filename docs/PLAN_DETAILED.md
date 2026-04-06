# Section MVP 详细任务拆分

> Archived note:
> this document is a pre-pivot, mount/FUSE-first detailed breakdown.
> It is kept as implementation history, not as the active roadmap.
> The active route is now `README.md`, `docs/PRODUCT.md`, `docs/SYNC_MODEL.md`, `docs/SECTIOND.md`, and `docs/PLAN.md`.

基于 PLAN.md 的状态分析和代码审查，将所有未完成工作拆分为最小独立功能单元。
每个任务可独立实现、独立测试、独立提交。

---

## 任务总览

| 分组 | 任务数 | 涉及 crate |
|------|--------|-----------|
| A. FUSE 缓存接入 | 10 | section-fuse, section-core |
| B. CLI refresh 接入 | 1 | section-cli |
| C. CLI 文件操作补全 | 4 | section-cli |
| D. 测试补全 | 5 | section-core, section-provider, section-fuse |
| E. mount 命令完善 | 3 | section-cli, section-fuse |
| F. 权限持久化 | 3 | section-core, section-fuse |
| G. 配置与文档 | 3 | section-core, 根目录 |
| H. 真实后端验证 | 2 | 集成测试 |
| I. 打包分发 | 2 | 根目录, section-fuse |
| **合计** | **33** | |

---

## A. FUSE 缓存接入

当前状态：`MetadataCache`（TTL）和 `ContentCache`（LRU）在 `section-core/src/cache.rs` 已实现含 23 个单元测试，但 `SectionFs` 完全未使用。

### A-1: SectionFs 添加 MetadataCache 和 ContentCache 字段并初始化

**文件**: `crates/section-fuse/src/fs.rs`
**内容**:
- `SectionFs` 结构体新增 `metadata_cache: MetadataCache` 和 `content_cache: ContentCache` 字段
- `SectionFs::new()` 中初始化，TTL 暂用全局常量（如 60s），max_bytes 暂用 64MB
- 不改变任何现有行为，仅添加字段

**验证**: 编译通过，现有 BDD 测试不退化

---

### A-2: lookup 接入 MetadataCache

**文件**: `crates/section-fuse/src/fs.rs`（`Filesystem::lookup`）
**内容**:
- 在 OpenDAL stat 调用前，先查 `metadata_cache.get_stat(child_path)`
- 命中则直接用缓存的 Metadata 构建 inode，跳过网络请求
- 未命中则走原有 OpenDAL stat 逻辑，成功后 `metadata_cache.put_stat(child_path, meta)`
- **不**改变 inode 缓存逻辑（InodeTable 仍保留）

**验证**: fs provider BDD 测试通过；手动观察第二次 lookup 不触发 OpenDAL 调用（通过 RUST_LOG=debug）

---

### A-3: readdir 接入 MetadataCache

**文件**: `crates/section-fuse/src/fs.rs`（`Filesystem::readdir`）
**内容**:
- 在 OpenDAL list 调用前，先查 `metadata_cache.get_listing(path)`
- 命中则用缓存的 `Vec<(String, Metadata)>` 构建目录项
- 未命中则走原有 list 逻辑，成功后 `metadata_cache.put_listing(path, entries)`
- 缓存的 listing 同时为每个子项填充 stat 缓存（`put_stat`）

**验证**: 连续两次 readdir 同一目录，第二次无 OpenDAL 网络调用

---

### A-4: open 读取时接入 ContentCache

**文件**: `crates/section-fuse/src/fs.rs`（`Filesystem::open`）
**内容**:
- 在 OpenDAL read 调用前，先查 `content_cache.get(path)`
- 命中则用缓存数据填充 `OpenFile.data`，跳过网络请求
- 未命中则走原有 read 逻辑，成功后 `content_cache.put(path, data.clone())`
- 注意：ContentCache::get 需要 `&mut self`（会 promote LRU），需要处理借用顺序

**验证**: 打开同一文件两次，第二次无 OpenDAL read 调用

---

### A-5: write-through 写入时同步更新缓存

**文件**: `crates/section-fuse/src/fs.rs`（`flush_fh`）
**内容**:
- `flush_fh` 成功写回 OpenDAL 后：
  - `content_cache.put(path, data.clone())` 更新内容缓存
  - `metadata_cache.invalidate(path)` 使 stat 缓存失效（size 已变）
- 保证 Section 自身写入后立即可读到最新内容

**验证**: 写入文件后立即 open 读取，得到最新数据且无额外网络请求

---

### A-6: create/mkdir 后失效父目录缓存

**文件**: `crates/section-fuse/src/fs.rs`（`Filesystem::create`, `Filesystem::mkdir`）
**内容**:
- `create` 成功后：`metadata_cache.invalidate(parent_path)` （使父目录 listing 缓存失效）
- `mkdir` 成功后：`metadata_cache.invalidate(parent_path)`
- 需要从 parent inode 反查 parent_path

**验证**: 在一个目录下创建文件后立即 readdir，能看到新文件

---

### A-7: unlink/rmdir/rename 后失效相关缓存

**文件**: `crates/section-fuse/src/fs.rs`（`Filesystem::unlink`, `rmdir`, `rename`）
**内容**:
- `unlink` 成功后：`metadata_cache.invalidate(path)` + `content_cache.remove(path)`
- `rmdir` 成功后：`metadata_cache.invalidate_prefix(path)` + 失效父目录 listing
- `rename` 成功后：失效旧路径 stat/content + 失效新/旧父目录 listing

**验证**: 删除文件后 readdir 不再包含该文件；重命名后 lookup 旧名返回 ENOENT

---

### A-8: 缓存 TTL/max_bytes 从配置读取，支持 per-source TTL

**文件**: `crates/section-fuse/src/fs.rs`
**内容**:
- `SectionFs::new()` 接收 `&SectionConfig`，从中读取各 source 的 `CacheConfig`
- MetadataCache 改为 per-source 实例（`HashMap<String, MetadataCache>`），每个 source 使用自己的 `metadata_ttl_secs`
- ContentCache 的 `max_bytes` 从全局配置读取（可在 `SectionConfig` 中新增 `cache_max_bytes` 字段，默认 64MB）
- `content_ttl_secs = 0` 或 `metadata_ttl_secs = 0` 时跳过缓存（如 local-fs 场景）

**文件**: `crates/section-core/src/config.rs`
**内容**:
- `SectionConfig` 新增可选的全局 `content_cache_max_bytes: Option<usize>`，默认 64MB

**依赖**: A-1 ~ A-7
**验证**: 配置不同 TTL 的两个 source，观察各自的缓存过期行为独立

---

### A-9: 后端不可达时的缓存降级

**文件**: `crates/section-fuse/src/fs.rs`（`lookup`, `readdir`, `open`）
**内容**:
- 当 OpenDAL 调用返回网络相关错误（非 NotFound/PermissionDenied）时：
  - 若 MetadataCache 有该路径的缓存（即使已过期），返回 stale data 而非 EIO
  - 若 ContentCache 有该文件内容，照常提供读取
- 添加 tracing::warn 日志标记降级行为
- 不改变 NotFound/PermissionDenied 等语义明确错误的处理

**依赖**: A-1 ~ A-8
**验证**: 断开网络后，之前访问过的文件/目录仍可读取（返回 stale 数据）

---

### A-10: FUSE 支持通过 xattr 触发缓存失效

**文件**: `crates/section-fuse/src/fs.rs`（新增 `Filesystem::getxattr` 实现）
**内容**:
- 实现 `getxattr`：当 xattr name 为 `section.refresh` 时，调用 `metadata_cache.invalidate(path)` + `content_cache.remove(path)`，返回 `b"ok"`
- 其他 xattr 返回 `ENODATA`
- 这样外部可通过 `getfattr -n section.refresh /mnt/section/path` 触发失效

**依赖**: A-1 ~ A-7
**验证**: 挂载后通过 getfattr 命令触发，观察后续访问重新从后端读取

---

## B. CLI refresh 接入

### B-1: CLI refresh 命令实现 + JSON 支持

**文件**: `crates/section-cli/src/cmd/file.rs`（`refresh` 函数）
**内容**:
- 函数签名添加 `json_mode: bool` 参数
- 检查挂载点是否存在（从 config.mount_point 读取）
- 若已挂载：通过 `xattr::get(mount_path/section_path, "section.refresh")` 触发 FUSE 缓存失效
- 若未挂载：CLI 无持久缓存，打印提示即可
- 替换当前的 TODO 空壳实现
- JSON 模式输出 `{"ok": true, "message": "Cache refreshed for ..."}`

**文件**: `crates/section-cli/src/main.rs`
**内容**:
- 更新 `Commands::Refresh` 的 handler 调用，传入 `json` 参数

**依赖**: A-10（FUSE xattr 支持）
**验证**: `section refresh work-s3/data/` 触发缓存刷新；`section refresh work-s3/ --json` 输出 JSON

---

## C. CLI 文件操作补全

### C-1: cp 支持本地路径（本地 ↔ source 互拷）

**文件**: `crates/section-cli/src/cmd/file.rs`（`cp` 函数）
**内容**:
- 判断路径是否为 source 路径：尝试 `Router::parse_path`，若返回 None 或 source 不存在则视为本地路径
- 本地 → source：`std::fs::read(local)` → `op.write(sub_path, data)`
- source → 本地：`op.read(sub_path)` → `std::fs::write(local, data)`
- 保留现有 source → source 逻辑不变

**验证**: `section cp /tmp/test.txt my-fs/test.txt` 和 `section cp my-fs/test.txt /tmp/out.txt` 均工作

---

### C-2: cp 支持递归目录复制

**文件**: `crates/section-cli/src/cmd/file.rs`（`cp` 函数或新增 `cp_recursive` 辅助函数）
**内容**:
- 添加 `--recursive` / `-r` 参数到 CLI `Cp` 命令定义（`crates/section-cli/src/main.rs`）
- 检测 src 是否为目录（通过 stat 或路径以 `/` 结尾）
- 递归 list src 目录 → 逐文件拷贝到 dst 对应子路径
- 目录自动 `create_dir`，文件 `read` + `write`

**验证**: `section cp -r my-fs/src-dir/ my-fs/dst-dir/` 完整复制目录树

---

### C-3: cat 支持大文件流式输出

**文件**: `crates/section-cli/src/cmd/file.rs`（`cat` 函数）
**内容**:
- 将 `op.read()` 一次性读取替换为 `op.reader()` 获取异步 reader
- 分块读取（如 64KB 块），每块立即写入 stdout
- 使用 `tokio::io::AsyncReadExt::read_buf` 或 `futures::AsyncReadExt`
- 保持对小文件的兼容（行为不变）

**验证**: `section cat my-fs/large-file.bin > /dev/null` 内存占用恒定，不随文件大小增长

---

### C-4: ls 支持 `-l` 长格式输出（mtime、mode、size）

**文件**: `crates/section-cli/src/main.rs`（`Commands::Ls` 定义）+ `crates/section-cli/src/cmd/file.rs`
**内容**:
- `Ls` 命令添加 `#[arg(short, long)] long: bool` 参数
- 默认模式保持当前简洁输出（仅 name + size），不破坏现有行为和 BDD 测试
- `-l` 模式启用详细输出：
  - 对每个 entry 额外获取 `last_modified()` 和 `mode()`（如果 OpenDAL Metadata 提供）
  - 文本格式：`drwxr-xr-x  2024-03-05 12:00  dirname/` 或 `-rw-r--r--  1024  2024-03-05  file.txt`
  - 若后端不提供 mtime，显示 `-`
- JSON 模式（无论是否 `-l`）添加 `mtime`、`mode` 字段

**验证**: `section ls` 简洁输出不变，`section ls -l my-fs/` 输出包含时间和权限信息

---

## D. 测试补全

### D-1: Router 路径解析单元测试

**文件**: `crates/section-core/src/router.rs`（新增 `#[cfg(test)] mod tests`）
**内容**:
- `parse_path("")` → None
- `parse_path("my-s3")` → Some { source: "my-s3", sub_path: "" }
- `parse_path("my-s3/docs/file.txt")` → Some { source: "my-s3", sub_path: "docs/file.txt" }
- `parse_path("/my-s3/docs/")` → 带前后 `/` 的处理
- `get_operator` 对不存在的 source 返回 SourceNotFound
- `from_config` 用 fs provider 构建成功

**验证**: `cargo test -p section-core -- router`

---

### D-2: Permission 权限检查单元测试

**文件**: `crates/section-core/src/permission.rs`（新增 `#[cfg(test)] mod tests`）
**内容**:
- owner 读/写/执行：mode 0o700 owner 全通，non-owner 不通
- group 读/写/执行：mode 0o070 同组通，非同组不通
- other 读/写/执行：mode 0o007 全通
- root (uid=0) 总是通过
- 组合模式：0o640 owner 读写、group 读、other 无
- `default_file()` / `default_dir()` / `account_root()` 返回预期 mode

**验证**: `cargo test -p section-core -- permission`

---

### D-3: ProviderStore CRUD 单元测试

**文件**: `crates/section-provider/src/store.rs`（新增 `#[cfg(test)] mod tests`）
**内容**:
- 在 tempdir 中 open store，验证表创建成功
- `add_source` + `list_sources` 往返一致
- `add_source` 同名覆盖（INSERT OR REPLACE）
- `remove_source` 后 list 不包含
- `load_all` 返回完整 SourceConfig（含 cache TTL）
- 加密验证：直接读 DB options_json 列，确认是密文非明文

**验证**: `cargo test -p section-provider -- store`

---

### D-4: Router resolve 集成单元测试

**文件**: `crates/section-core/src/router.rs`（在 D-1 基础上扩展）
**内容**:
- 构建含两个 fs source 的 Router
- `resolve("source1/a.txt")` 返回正确的 Operator + sub_path
- `resolve("nonexistent/a.txt")` 返回 SourceNotFound
- `resolve("")` 返回 InvalidPath
- `sources()` 返回排序后的 source 列表
- `add_operator` 后 `get_operator` 能获取到

**验证**: `cargo test -p section-core -- router`

---

### D-5: FUSE 集成测试

**文件**: `crates/section-fuse/tests/integration.rs`（新建）
**内容**:
- 使用 fs provider 在 tmpdir 中构建 Router
- 用 `fuser::spawn_mount2` 挂载到临时目录
- 通过标准文件系统 API 测试：
  - `std::fs::read_dir` 列出 source 目录
  - `std::fs::write` + `std::fs::read` 写入和读取文件
  - `std::fs::create_dir` 创建子目录
  - `std::fs::remove_file` 删除文件
- 测试结束后 unmount 并清理
- 需要检测 fuse 内核模块是否可用，不可用则 skip（CI 环境可能无 FUSE）

**验证**: `cargo test -p section-fuse -- integration`（需 root 或 fuse 权限）

---

## E. mount 命令完善

### E-1: mount 命令传递 config 路径和数据目录给 section-fuse

**文件**: `crates/section-cli/src/cmd/mount.rs`（`mount` 函数）
**内容**:
- 将 `config.data_dir` 通过 `--data-dir` 参数传递给 section-fuse 子进程
- 若用户指定了 `--config`，同样通过 `--config` 参数传递给 section-fuse（当前仅检查 `SECTION_CONFIG` 环境变量，不可靠）
- 这样 section-fuse 能找到 SQLite 数据库和配置文件

**文件**: `crates/section-fuse/src/main.rs`
**内容**:
- 添加 `--data-dir` CLI 参数
- 若提供 `--data-dir` 则用指定路径打开 ProviderStore，否则从 config 读取

**验证**: `section --config /tmp/test.toml mount` 启动的 section-fuse 能正确加载配置和 DB 中的 source

---

### E-2: mount 命令等待挂载就绪

**文件**: `crates/section-cli/src/cmd/mount.rs`
**内容**:
- spawn section-fuse 后，轮询检查挂载点是否就绪（检查 `/proc/mounts` 或 `stat` 挂载点）
- 设置超时（如 5 秒），超时则报错
- 成功后打印挂载信息

**验证**: `section mount` 等到 FUSE 实际就绪后才返回

---

### E-3: mount/unmount 的 JSON 输出支持

**文件**: `crates/section-cli/src/cmd/mount.rs` + `crates/section-cli/src/main.rs`
**内容**:
- `mount` 函数签名添加 `json_mode: bool` 参数
- `unmount` 函数签名添加 `json_mode: bool` 参数
- JSON 模式输出 `{"ok": true, "mount_point": "/mnt/section"}`
- 更新 `main.rs` 中的调用传入 `json` 参数

**验证**: `section mount --json` 输出 JSON

---

## F. 权限持久化

### F-1: SQLite 权限表定义和 CRUD

**文件**: `crates/section-provider/src/store.rs`（或新建 `crates/section-provider/src/permission_store.rs`）
**内容**:
- 新增 `permissions` 表：`(path TEXT PRIMARY KEY, uid INTEGER, gid INTEGER, mode INTEGER)`
- `set_permission(path, uid, gid, mode)` → INSERT OR REPLACE
- `get_permission(path)` → Option<Permission>
- `delete_permission(path)` → 删除
- `delete_permissions_prefix(prefix)` → 删除前缀匹配的所有权限

**验证**: 单元测试 CRUD 操作

---

### F-2: FUSE setattr 时持久化权限

**文件**: `crates/section-fuse/src/fs.rs`
**内容**:
- `SectionFs` 新增 `ProviderStore` 引用（或独立的权限存储句柄）
- `setattr` 中若 mode/uid/gid 被修改，将新值持久化到 SQLite
- 仅对 source 内路径持久化（root 和 source 顶层目录使用默认权限）

**依赖**: F-1
**验证**: `chmod 600 /mnt/section/my-fs/secret.txt` 后重启 FUSE 进程，权限仍为 600

---

### F-3: FUSE 启动时加载持久化权限

**文件**: `crates/section-fuse/src/fs.rs`（`SectionFs::new` 或 lookup/ensure 逻辑）
**内容**:
- inode 创建时（`ensure`）查询 SQLite 是否有持久化的权限
- 若有则使用持久化值替代默认的 0o644/0o755
- 若无则使用默认值（当前行为不变）

**依赖**: F-1, F-2
**验证**: 重启 FUSE 后，之前 chmod 的文件保留自定义权限

---

## G. 配置与文档

### G-1: 配置校验

**文件**: `crates/section-core/src/config.rs`
**内容**:
- 新增 `SectionConfig::validate()` 方法
- 校验规则：
  - `mount_point` 必须是绝对路径
  - `data_dir` 必须是绝对路径
  - source name 不能包含 `/` 或空白字符
  - provider 必须是已知类型（fs, s3, webdav, 或 OpenDAL 支持的 scheme）
- 在 `SectionConfig::load()` 成功解析后调用 validate

**验证**: 单元测试非法配置返回明确错误

---

### G-2: config.example.toml 示例配置

**文件**: `config.example.toml`（项目根目录）
**内容**:
```toml
# Section 配置文件示例
# 默认路径: ~/.config/section/config.toml

mount_point = "/mnt/section"
data_dir = "/home/user/.local/share/section"

[sources.local-workspace]
provider = "fs"
[sources.local-workspace.options]
root = "/home/user/workspace"
[sources.local-workspace.cache]
metadata_ttl_secs = 0
content_ttl_secs = 0

[sources.work-s3]
provider = "s3"
[sources.work-s3.options]
bucket = "my-bucket"
region = "us-east-1"
access_key_id = "AKIA..."
secret_access_key = "..."
[sources.work-s3.cache]
metadata_ttl_secs = 60
content_ttl_secs = 300

[sources.office-nas]
provider = "webdav"
[sources.office-nas.options]
endpoint = "https://nas.local/dav"
username = "admin"
password = "..."
```

**验证**: `section --config config.example.toml source list` 能解析（连接会失败但不 panic）

---

### G-3: README 补充安装和配置章节

**文件**: `README.md`
**内容**:
- 补充 `cargo install --path crates/section-cli` 安装说明
- 补充配置文件路径说明和 `config.example.toml` 引用
- 补充 FUSE 依赖说明（`sudo apt install fuse3` 或内核模块）
- 补充常见问题（mount 权限、allow_other 需要 /etc/fuse.conf 配置）

**验证**: 按 README 步骤操作能成功构建和运行

---

## H. 真实后端验证

### H-1: S3 兼容存储端到端验证

**环境**: MinIO 容器或 AWS S3
**内容**:
- `section source add test-s3 --provider s3 --opt bucket=... --opt region=... --opt access_key_id=... --opt secret_access_key=...`
- 验证 `section ls test-s3/`
- 验证 `section write test-s3/test.txt` + `section cat test-s3/test.txt`
- 验证 `section cp test-s3/test.txt test-s3/copy.txt`
- 验证 `section rm test-s3/test.txt`
- 记录 OpenDAL S3 provider 的特殊行为或所需额外 options

**交付物**: 测试记录 + 必要的代码修复

---

### H-2: WebDAV 端到端验证

**环境**: WebDAV 服务器（如 rclone serve webdav 或 Nextcloud）
**内容**:
- `section source add test-dav --provider webdav --opt endpoint=... --opt username=... --opt password=...`
- 同 H-1 的 CRUD 验证步骤
- 记录 WebDAV provider 的特殊行为

**交付物**: 测试记录 + 必要的代码修复

---

## I. 打包分发

### I-1: cargo install 支持

**文件**: `crates/section-cli/Cargo.toml`, `crates/section-fuse/Cargo.toml`, 根 `Cargo.toml`
**内容**:
- 确认 `[package]` 中 `name`、`version`、`description`、`license`、`repository` 等元数据完整
- 确认 `[[bin]]` 定义正确（section-cli → `section`，section-fuse → `section-fuse`）
- 验证 `cargo install --path crates/section-cli` 和 `cargo install --path crates/section-fuse` 均成功
- 如有需要添加 `[package.metadata]` 或调整 feature flags

**验证**: 在干净环境中 `cargo install --path crates/section-cli` 成功，`section --help` 正常输出

---

### I-2: systemd service 文件

**文件**: `contrib/section-fuse.service`（新建）
**内容**:
```ini
[Unit]
Description=Section FUSE filesystem daemon
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/section-fuse --mount-point /mnt/section
ExecStop=/bin/fusermount3 -u /mnt/section
Restart=on-failure

[Install]
WantedBy=multi-user.target
```
- 附带简要安装说明（`systemctl enable --now section-fuse`）

**验证**: `systemd-analyze verify contrib/section-fuse.service` 无警告

---

## 执行顺序建议

```
优先级 P0（核心链路）:
  A-1 → A-2 → A-3                    # 元数据缓存接入（含 ContentCache 字段初始化）
  A-1 → A-4                          # 内容缓存接入
  A-5 → A-6 → A-7                    # write-through + 失效
  C-1                                # cp 本地路径（最常用功能缺口）

优先级 P1（体验改善）:
  C-2, C-3, C-4                      # CLI 操作补全（互相独立）
  E-1 → E-2                          # mount 可靠性
  A-8                                # per-source TTL 可配置
  A-10 → B-1                          # refresh 接入
  D-1, D-2, D-3, D-4, D-5           # 测试补全（D-1~D-4 互相独立，D-5 独立）

优先级 P2（完善）:
  A-9                                # 后端不可达缓存降级
  E-3                                # mount JSON 输出
  F-1 → F-2 → F-3                    # 权限持久化
  G-1, G-2, G-3                      # 配置与文档
  H-1, H-2                           # 真实后端验证
  I-1, I-2                           # 打包分发
```

### 依赖关系图

```
A-1 ──→ A-2 ──→ A-5
  │       │       │
  │       │       ├──→ A-6
  │       │       └──→ A-7
  │       └──→ A-3
  └──→ A-4 ──→ A-5

A-7 ──→ A-8（per-source TTL）
A-7 ──→ A-9（缓存降级）
A-7 ──→ A-10 ──→ B-1

C-1, C-2, C-3, C-4 全部互相独立

D-1, D-2, D-3, D-4 全部独立
D-5 独立（需 FUSE 权限）

E-1 ──→ E-2
E-3 独立

F-1 ──→ F-2 ──→ F-3

G-1, G-2, G-3 全部独立

H-1, H-2 独立（建议在 C-1 之后做）

I-1 独立
I-1 ──→ I-2
```

---

## 工作量估算

| 任务 | 改动规模 | 核心文件 |
|------|---------|---------|
| A-1 | ~15 行 | fs.rs |
| A-2 | ~30 行 | fs.rs (lookup) |
| A-3 | ~40 行 | fs.rs (readdir) |
| A-4 | ~25 行 | fs.rs (open) |
| A-5 | ~15 行 | fs.rs (flush_fh) |
| A-6 | ~20 行 | fs.rs (create, mkdir) |
| A-7 | ~25 行 | fs.rs (unlink, rmdir, rename) |
| A-8 | ~60 行 | fs.rs + config.rs |
| A-9 | ~40 行 | fs.rs (lookup, readdir, open) |
| A-10 | ~30 行 | fs.rs (getxattr) |
| B-1 | ~30 行 | file.rs (refresh) + main.rs |
| C-1 | ~50 行 | file.rs (cp) |
| C-2 | ~60 行 | file.rs + main.rs |
| C-3 | ~30 行 | file.rs (cat) |
| C-4 | ~50 行 | file.rs (ls) + main.rs |
| D-1 | ~60 行 | router.rs (tests) |
| D-2 | ~80 行 | permission.rs (tests) |
| D-3 | ~80 行 | store.rs (tests) |
| D-4 | ~50 行 | router.rs (tests) |
| D-5 | ~100 行 | section-fuse/tests/integration.rs |
| E-1 | ~25 行 | mount.rs + fuse/main.rs |
| E-2 | ~30 行 | mount.rs |
| E-3 | ~20 行 | mount.rs + main.rs |
| F-1 | ~60 行 | store.rs / 新文件 |
| F-2 | ~30 行 | fs.rs (setattr) |
| F-3 | ~25 行 | fs.rs (ensure) |
| G-1 | ~40 行 | config.rs |
| G-2 | ~30 行 | config.example.toml |
| G-3 | ~50 行 | README.md |
| H-1 | 测试 | - |
| H-2 | 测试 | - |
| I-1 | ~20 行 | Cargo.toml (多个) |
| I-2 | ~20 行 | contrib/section-fuse.service |
