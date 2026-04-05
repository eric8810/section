# Section - Agent-First Unified Data Layer

## 一句话定义

Section 是一个面向 AI Agent 优先的统一数据访问层，将多个存储源以标准文件系统的形式暴露给本地环境，目标同时支持 macOS 和 Linux，并通过 CLI 和 GUI 后端供人类管理。

## 核心理念

Agent 和人类通过同一套文件系统语义访问任意数据源，无需关心底层存储是 S3、Google Drive、NAS 还是本地磁盘。

## 数据模型

```
Provider (类型定义)                      Source (实例)
  ├── aws-s3                            ├── work-s3         (provider: aws-s3)
  │     认证: access_key + secret_key   │     bucket=work-files, region=us-east-1
  ├── google-drive                      ├── my-gdrive       (provider: google-drive)
  │     认证: OAuth token               │     OAuth token bound
  ├── webdav                            ├── office-nas      (provider: webdav)
  │     认证: url + username + password  │     url=https://nas.local/dav
  └── local-fs                          └── local-workspace (provider: fs)
        认证: root path                       root=/home/user/workspace
```

Provider 定义平台类型和认证方式。Source 是 Provider 的实例，绑定了用户提供的具体凭证。
Source 的权限由凭证本身决定——凭证能做什么，Source 就能做什么。Section 只透传。

### 挂载结构

```
/mnt/section/                           ← 统一挂载点
├── work-s3/                            ← source: work-s3 (provider: aws-s3)
│   ├── projects/
│   └── backups/
├── my-gdrive/                          ← source: my-gdrive (provider: google-drive)
│   └── documents/
├── office-nas/                         ← source: office-nas (provider: webdav)
│   └── shared/
└── local-workspace/                    ← source: local-workspace (provider: fs)
    └── code/
```

## 目标用户

| 优先级 | 用户 | 访问方式 |
|--------|------|---------|
| P0 | AI Agent | 直接读写挂载路径 (`open("/mnt/section/work-s3/data.csv")`) |
| P1 | 开发者/运维 | CLI (`section ls work-s3/projects/`) |
| P2 | 普通用户 | GUI (桌面/Web，通过后端 API) |

## 核心模块

### 1. section-fuse (挂载守护进程)

将多数据源聚合为一个 FUSE 文件系统。

职责：
- 挂载统一命名空间到 `/mnt/section/`
- 路径路由: 解析路径 → 确定 (source, sub-path) → 调用 OpenDAL
- 权限执行: 基于 POSIX 风格权限模型 (当前实现更偏 Linux) 做访问控制
- 文件缓存: 热文件本地缓存，减少远端请求
- 执行支持: 允许执行挂载路径上的文件 (有 x 权限即可执行)

### 2. section-cli (命令行工具)

人类管理入口。

```bash
# 数据源管理
section source add work-s3 --provider s3 \
    --opt bucket=my-bucket --opt region=us-east-1 \
    --opt access_key_id=xxx --opt secret_access_key=xxx
section source add my-gdrive --provider gdrive --opt ...
section source add office-nas --provider webdav \
    --opt endpoint=https://nas.local/dav --opt username=admin --opt password=xxx
section source list
section source remove work-s3

# 文件操作
section ls                              # 列出所有 source
section ls work-s3/projects/
section cp work-s3/report.pdf office-nas/shared/
section cat work-s3/config.yaml
section rm work-s3/tmp/ -r

# 挂载管理
section mount                           # 挂载到 /mnt/section/
section mount --path /custom/mount
section unmount

# 缓存
section refresh work-s3/data/
```

### 3. section-provider (数据源管理服务)

管理 Provider 定义和 Source 实例。

职责：
- Source 实例管理: 创建/删除/列出
- 凭证存储: 加密存储 API Key、OAuth Token 等
- OAuth 流程: 处理 Google/Microsoft/Dropbox 等 OAuth 授权
- Token 刷新: 自动刷新过期的 OAuth Token
- 未来扩展: SSO 对接 (OIDC/SAML)

存储: 本地 SQLite (单机)

### 4. section-core (核心库)

底层逻辑，被以上所有模块依赖。

职责：
- OpenDAL 集成: 管理多个 Operator 实例，按 source name 索引
- 路径路由器: 解析路径 `{source}/{sub_path}`，分发到对应的 Operator
- 权限模型: POSIX 风格权限语义的实现与持久化（当前实现更偏 Linux）
- 缓存管理: LRU 缓存策略，缓存元数据与文件内容

## 权限模型

基于 POSIX 风格权限体系，在 FUSE 层执行；当前实现和验证深度仍明显偏 Linux。

### 设计原则

- 每个文件/目录有 owner (uid), group (gid), mode (rwx)
- 权限信息存储在 section 自身的元数据库中 (非底层存储)
- FUSE 进程以 root 运行，根据访问者的 uid/gid 做权限检查
- 执行远端文件: 有 x 权限即可执行，安全由文件系统权限管控

### Agent 访问

Agent 进程以某个系统用户身份运行，自然继承该用户的文件系统权限。
无需特殊 Agent 鉴权——标准 Unix 权限即是鉴权。

### 跨平台适配

| 平台 | 权限实现 | 优先级 |
|------|---------|--------|
| Linux | 原生 FUSE + POSIX 权限 | Phase 1 (validated first) |
| macOS | macFUSE + POSIX 风格权限语义 | Phase 1 target (mount validation ongoing) |
| Windows | WinFSP + ACL 映射 | Phase 3 |

## 缓存与一致性策略

### 原则

- Section 自身读写 → 强一致 (write-through)
- 外部变更 → 可配置 TTL，按数据源设定
- 提供 `section refresh` 命令兜底
- MVP 不做实时监听 (S3 Event / webhook 留后续版本)

### 默认 TTL

```yaml
sources:
  work-s3:
    cache:
      metadata_ttl: 60s       # 目录列表缓存 60 秒
      content_ttl: 300s        # 文件内容缓存 5 分钟

  my-gdrive:
    cache:
      metadata_ttl: 30s
      content_ttl: 120s

  local-workspace:
    cache:
      metadata_ttl: 0          # 本地磁盘不缓存
      content_ttl: 0
```

### 强制刷新

```bash
section refresh work-s3/data/
# FUSE 层通过 xattr 支持强制失效
# macOS:
xattr -p section.refresh /mnt/section/work-s3/important.csv
# Linux:
getfattr -n user.section.refresh /mnt/section/work-s3/important.csv
```

## 技术选型

| 组件 | 选型 | 理由 |
|------|------|------|
| 存储抽象 | Apache OpenDAL | Rust 原生, Apache 2.0, 60+ 后端 |
| FUSE | fuser | 统一 Rust 接口，但运行时前提仍受平台影响 |
| 元数据库 | SQLite (rusqlite, bundled) | 嵌入式, 零运维 |
| 凭证加密 | ring | 本地加密存储敏感凭证 |
| CLI 框架 | clap | Rust 生态标准 |
| 异步运行时 | tokio | OpenDAL 依赖 tokio |

## 许可证

Apache License 2.0 — 商业友好，与 OpenDAL 一致。

## 与现有项目的关系

| 项目 | 关系 |
|------|------|
| OpenDAL | 核心依赖 — 存储抽象层 |
| ofs | 参考但不直接使用 — ofs 只支持单后端, section-fuse 需要多后端路由 |
| rclone | 非竞品 — rclone 是工具, section 是数据层 |
| Spacedrive | 理念有交集 (VDFS), 但 section 面向 Agent, 更轻量, 更聚焦 |
| AList/OpenList | 都做多存储聚合，但 section 提供文件系统挂载，不只是 Web 浏览 |

## MVP 范围 (Phase 1)

目标: 单机可用的 Agent 数据层。Phase 1 先把 macOS + Linux 的 non-FUSE 路径一起做实，再分别验证挂载路径。

包含:
- [ ] section-core: OpenDAL 多后端管理 + 路径路由
- [ ] section-fuse: FUSE 挂载, 支持 read/write/readdir/stat/exec
- [x] section-cli: source/ls/cp/cat/rm/mount/unmount/refresh 命令
- [ ] section-provider: 本地 SQLite 存储, 手动凭证配置
- [ ] 权限: 基础 POSIX mode 执行
- [ ] Provider 支持: fs, s3, webdav
- [ ] 双平台支持: macOS + Linux 的 non-FUSE 路径保持可用，挂载路径分别验证并逐步收敛

不包含:
- GUI
- 团队/多机同步
- SSO / OAuth 自动流程
- MCP Server
- Windows 支持
