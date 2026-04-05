# Section MVP 落地计划

## 当前状态

当前仓库更接近“跨平台目标下的工程原型”，还不是已经做实的双平台 MVP。
已验证的事实：
- `section-cli` BDD 测试 32/32 场景通过
- `section-core` / `section-provider` 当前单元测试共 36/36 通过
- `docs/BACKEND_VALIDATION.md` 已记录 S3 与 WebDAV 的可重复本地验证流程
- 当前 macOS 环境下，整仓 FUSE 路径还不能被视为已验证完成

### 各模块完成度

| 模块 | 状态 | 说明 |
|------|------|------|
| **section-core/config** | 90% | 配置加载/序列化可用，缺少配置校验 |
| **section-core/router** | 100% | 路径解析、Operator 构建、排序与错误路径单元测试已补齐 |
| **section-core/permission** | 85% | 数据结构和检查逻辑完整，已接入 FUSE，未持久化到 SQLite，关键权限单元测试已补齐 |
| **section-core/cache** | 90% | MetadataCache + ContentCache 已接入 FUSE，并支持 refresh/xattr 失效 |
| **section-provider/store** | 95% | SQLite CRUD、AES-256-GCM 加密存储、脱敏显示、明文迁移测试已就位 |
| **section-provider/crypto** | 100% | AES-256-GCM 加解密，密钥自动生成/加载，3 个单元测试 |
| **section-provider/oauth** | 0% | 未实现（Phase 2） |
| **section-fuse/fs** | 90% | 完整 FUSE 实现（读写删改名权限），metadata/content cache 与 refresh 已接入，缺真实挂载验证 |
| **section-fuse/inode** | 100% | 动态 inode 分配、lookup 缓存、父子关系追踪 |
| **section-cli/source** | 100% | add/remove/list + 脱敏 + JSON |
| **section-cli/file** | 95% | ls/cp/cat/rm/write/exec 可用，已补齐 `ls -l`、本地/remote copy、递归 copy、cat 流式输出 |
| **section-cli/mount** | 35% | 调用 section-fuse 子进程，已开始补平台差异处理，但双平台挂载尚未验证 |
| **section-cli/init** | 100% | 交互式引导创建 source |
| **section-cli/status** | 100% | 挂载状态 + 连通性探测 + JSON |
| **测试** | 85% | BDD 32 场景 + 40 单元测试，仍缺真实后端与真实挂载路径验证 |
| **文档** | 80% | PRODUCT.md + README.md 完成，缺 config.example.toml |

---

## 落地计划

### Phase 1A: 核心链路打通（CLI 可用）

目标：`section source add` + `section ls/cp/cat` 能跑通真实存储。

#### 1. 验证 OpenDAL 对接 [P0]
- [x] 用本地文件系统 (provider: fs) 跑通完整 CRUD
- [x] 用 S3 兼容存储 (Moto 本地端点) 跑通完整 CRUD
- [x] 用 WebDAV (WsgiDAV + Cheroot) 跑通完整 CRUD
- [x] 确认 OpenDAL API 在实际使用中的坑（via_iter 类型、write 空 Vec 歧义等）

#### 2. CLI 文件操作补全 [P0]
- [x] `section ls` 输出优化（大小、时间、权限的格式化显示）— 已支持 `ls -l`
- [x] `section cp` 支持递归复制目录
- [x] `section cp` 支持本地路径 ↔ source 路径互拷
- [x] `section cat` 支持大文件流式输出
- [x] `section exec` 实现（下载到临时文件 → chmod +x → 执行 → 清理）
- [x] `section write` 或管道写入支持（`echo "data" | section write work-s3/file.txt`）

#### 3. 凭证安全 [P1]
- [x] 凭证加密存储（ring AES-256-GCM 加密后写入 SQLite）
- [x] 敏感字段在 `section source list` 中脱敏显示

#### 4. 基础测试 [P1]
- [x] section-core: Router 路径解析单元测试
- [x] section-core: Permission 权限检查单元测试
- [x] section-provider: ProviderStore CRUD / 迁移 单元测试
- [x] section-cli: 集成测试（用 fs provider 做端到端，32 个 BDD 场景）

**交付物：一个可用的 CLI 工具，能管理多 source 并执行文件操作。** ✅ 基本达成

---

### Phase 1B: FUSE 挂载可用（Linux + macOS 分别验证）

目标：`section mount` 后，agent 和人类能通过文件系统路径直接访问。

#### 5. FUSE 完整实现 [P0]
- [x] inode 管理系统（InodeTable 动态分配）
- [x] 递归目录浏览（root → source → 子目录 → 文件，逐层代理到 OpenDAL）
- [x] 文件读取 (open → 全量读入缓冲 → offset/size 服务 read)
- [x] 文件写入 (write → 缓冲 → flush/release 时写回 OpenDAL)
- [x] 创建文件/目录 (create/mkdir → OpenDAL write/create_dir)
- [x] 删除文件/目录 (unlink/rmdir → OpenDAL delete)
- [x] 文件属性 (getattr → inode 缓存，setattr 支持 size/mode/uid/gid)
- [x] 重命名 (rename → OpenDAL copy + delete，跨 source 返回 EXDEV)

#### 6. 缓存层 [P0]
- [x] 元数据缓存（MetadataCache：stat + listing 结果，TTL 过期，11 个单元测试）
- [x] 内容缓存（ContentCache：LRU 淘汰，12 个单元测试）
- [x] **缓存接入 FUSE**：`lookup` / `readdir` / `open` 已接入 metadata/content cache
- [x] write-through：Section 写入回刷后同步更新内容缓存并失效元数据缓存
- [x] `section refresh` 接入缓存失效：挂载路径通过 xattr 触发 FUSE 侧缓存清理，CLI 未挂载时返回明确 no-op 提示

#### 7. 权限接入 FUSE [P1]
- [ ] 权限元数据持久化到 SQLite — 当前仅内存中
- [x] FUSE getattr 返回正确的 uid/gid/mode
- [x] FUSE 操作前检查请求者的 uid/gid 权限（open/create/mkdir/unlink/rmdir/rename）
- [x] 默认权限策略（dirs: 0o755, files: 0o644, owner: 进程 uid/gid）

#### 8. 错误处理 [P1]
- [x] FUSE 操作的错误码映射（opendal_to_errno: NotFound→ENOENT 等）
- [ ] 后端不可达时的优雅降级（缓存可用则返回缓存）— 缓存未接入
- [x] 日志框架（tracing + EnvFilter，RUST_LOG 控制）

**交付物：一个可挂载的文件系统，agent 可通过标准文件操作访问所有 source。** ⚠️ 基本达成，缓存未接入是主要缺口

---

### Phase 1C: 打磨发布

目标：可以给别人用。

#### 9. 配置与用户体验 [P1]
- [ ] 示例配置文件（config.example.toml）
- [x] `section init` 命令（交互式引导创建首个 source，支持 fs/s3/webdav）
- [x] `section status` 命令（挂载状态 + source 连通性探测）
- [x] 友好的错误提示（from_opendal 包装，不暴露堆栈）
- [x] `--json` 输出模式（全命令覆盖，方便 agent 解析）

#### 10. 文档 [P1]
- [x] README.md（项目介绍、快速开始、架构图）
- [x] 支持的 Provider 列表及配置示例（在 README 中）
- [x] 安装说明（cargo build，基础可用）

#### 11. 打包 [P2]
- [ ] cargo install 支持
- [ ] GitHub Release 二进制
- [ ] AUR / Homebrew / Nix 包（按需）
- [ ] systemd service 文件（section-fuse 开机自启）

**交付物：可安装、有文档、别人能用的 0.1.0 版本。** ⚠️ 功能基本齐全，打包未做

---

### Phase 2: 扩展（MVP 之后）

| 功能 | 说明 |
|------|------|
| OAuth 自动流程 | Google/Microsoft/Dropbox 一键授权 |
| MCP Server | Agent 通过 MCP 协议访问（不依赖 FUSE） |
| macOS 支持 | macFUSE 路径验证与命令差异收敛 |
| 同步引擎 | 本地 ↔ 远端双向同步 |
| GUI 后端 | REST API 供桌面/Web 客户端调用 |
| 更多 Provider | FTP, OneDrive, 阿里云 OSS, 百度网盘... |
| 插件系统 | 可扩展的 Provider 插件 |

---

## 未完成事项清单

以下是 MVP 阶段尚未完成的工作，按优先级排列：

### 高优先 — 影响核心功能
1. **缓存接入 FUSE**：MetadataCache/ContentCache 类已就绪，需在 SectionFs 的 lookup/readdir/open 中实际使用
2. **refresh 命令接入**：调用 MetadataCache::invalidate() 使缓存失效
3. **S3 实际验证**：用 MinIO 或 AWS S3 端到端测试
4. **WebDAV 实际验证**：用 WebDAV 服务端到端测试

### 中优先 — 改善体验
5. **cp 递归复制**：遍历目录 + 逐文件拷贝
6. **cp 本地路径支持**：检测非 source 路径时直接读写本地文件
7. **cat 流式输出**：使用 OpenDAL reader + 分块写入 stdout
8. **ls 输出增强**：显示 mtime、mode 列
9. **config.example.toml**：带注释的示例配置

### 低优先 — 补充测试
10. **Router 单元测试**：parse_path、resolve 等
11. **Permission 单元测试**：can_read/can_write/can_execute 各场景
12. **ProviderStore 单元测试**：add/remove/list/load_all
13. **权限持久化**：权限元数据写入 SQLite 在重启后保留

### Phase 2 范围
14. 打包分发（cargo install、GitHub Release、systemd service）
15. 后端不可达时的缓存降级
16. OAuth 自动授权流程
17. MCP Server

---

## 提交历史

```
4e35ade Add README with quick start, architecture, and usage docs
ccc9422 Add content cache, section init, and --json output mode
fa0b514 Add metadata cache, FUSE permissions, and section status command
dd965e4 Implement full FUSE filesystem with dynamic inodes
5372b66 Add credential masking and AES-256-GCM encrypted storage
e196f83 Add .gitignore, remove target/ from tracking
795671a Phase 1A: exec, write, error handling - 27/27 BDD green
8501c2e Initial project skeleton
```
