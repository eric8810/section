# Section AgentFS Design Proposal

## Purpose

This document proposes the first concrete design for Section AgentFS.

It implements the product direction in [AGENTFS_REQUIREMENTS.md](AGENTFS_REQUIREMENTS.md) while keeping the first version narrow. The concrete MVP development contract lives in [AGENTFS_MVP_CONTRACT.md](AGENTFS_MVP_CONTRACT.md).

## Design Thesis

Section AgentFS is built around one boundary:

```text
Local edits are drafts.
Accepted commits are shared filesystem truth.
```

The first design should not try to govern every local write. A normal attached directory can be edited by local tools, shells, humans, and agents. Section governs whether those edits are accepted into shared truth.

## 服务端控制面

正式产品只有一条接入路径：

```text
跨机 / 多 agent 分享必须走 Section Control Service。
```

服务端负责：

- agent 身份
- installation 身份
- FS 发现
- grant 管理
- share 管理
- source profile 管理
- 短期同步 credential 发放
- 审计

本地 AgentFS 负责：

- login 后缓存身份
- attach 到本地目录
- 扫描 local edits
- status/watch
- commit apply
- 用服务端发的 credential 运行 sync layer

明确不做：

- invite file
- 离线分享
- 手动 source bootstrap
- 分享 source 长期密钥
- 没有服务端的跨机分享

## Truth Authority

The authority model is:

```text
Section Control Service = identity, grant, share, source profile, credential authority
accepted commit log = governance truth
backing source = materialized filesystem state
local mount = working copy
```

An accepted commit records what became true, who committed it, what paths changed, and which grant or policy allowed it.

The backing source is the materialized state used for sync and normal file access. If a commit is accepted but backing-source sync fails, the commit remains the governance record and the FS enters `syncing` or `error` until materialization catches up.

This avoids ambiguity between "the database says the commit happened" and "the remote files finished syncing".

Grant, share, source profile, and credential authority do not live in the backing source. They live in Section Control Service.

## MVP Workflow

```text
agent logs in
agent creates fs
Section Control Service binds FS to a source profile
agent owns fs
agent grants another agent access
agent shares fs through the control service
other agent accepts the server-side share
Section Control Service issues sync credential
other agent attaches fs as a local directory
other agent edits files locally
other agent commits changes
Section checks grant and freshness
accepted commit updates governance truth
Section materializes accepted state to the backing source
agents observe the accepted mutation
```

## MVP Boundary

Included in the first core:

- FS creation and ownership
- agent identity
- installation identity
- server-side source profiles
- grant-based attach and commit authority
- server-side share and accept
- server-issued sync credentials
- attach to a local directory
- local edits as uncontrolled drafts
- commit as the governed mutation boundary
- accepted commit records
- observable commit and state events

Deferred layers:

- hooks and automation
- `AGENTS.md` enforcement
- approval workflows
- path-scoped grants
- multi-branch or fork UI
- semantic search or memory
- sandbox/runtime orchestration

## Core Objects

| Object | Meaning |
| --- | --- |
| `Agent` | Logical actor authenticated by Section Control Service |
| `Installation` | One local runtime/machine for an agent |
| `FS` | Agent-owned shared filesystem bound to a source profile |
| `SourceProfile` | Server-owned description of the remote backing source |
| `Share` | Server-side record that lets a granted agent discover and accept an FS |
| `Credential` | Short-lived sync credential issued after grant check |
| `Mount` | Local directory attached to an FS on one installation |
| `Grant` | Server-side permission from one agent to another for an FS |
| `Commit` | Accepted mutation that advances shared FS truth |
| `Event` | Observable record of accepted mutations and state changes |

Deferred objects:

| Object | Why deferred |
| --- | --- |
| `Rule` / `AGENTS.md` policy | Policy layer after the authority boundary is proven |
| `Hook` | Automation layer after hook trust and execution boundaries are defined |
| `Change` / proposal | Approval workflow after direct commit is implemented |

## Authority Model

An FS has exactly one owner at creation time.

The owner can:

- attach
- commit
- grant access
- revoke access
- inspect events
- manage the FS

Other agents can act only through grants.

MVP capabilities:

| Capability | Allows |
| --- | --- |
| `read` | attach and read committed FS truth |
| `commit` | submit local changes for acceptance into shared truth |
| `manage` | grant/revoke access and manage FS metadata |

MVP roles:

| Role | Capabilities |
| --- | --- |
| `owner` | `read`, `commit`, `manage` |
| `reader` | `read` |
| `writer` | `read`, `commit` |
| `manager` | `read`, `manage` |

There is no separate MVP `write` capability. Local writes are controlled by the local operating environment, not by Section. Section controls whether local writes can become accepted shared truth.

## Control Metadata

AgentFS control metadata must be shared across agents.

Local-only SQLite is not enough for:

- grants
- accepted commits
- FS events
- ownership
- current FS head

For the product, Section Control Service is authoritative for:

- agent identity
- installation identity
- grants
- shares
- source profiles
- credential issuance
- FS discovery

The accepted commit log remains the filesystem truth record. It may be stored in the service, mirrored into the backing source, or both. Product behavior must not depend on the backing source being the authority for access control.

The backing source may still contain a mirror namespace:

```text
.section/
  agentfs/
    fs.json
    heads/
      current.json
    grants/
      <grant_id>.json
    commits/
      <commit_id>.json
    events/
      <event_id>.json
```

This namespace is for materialization, audit, repair, and local diagnosis. It is not the authority for cross-machine grant/share/credential decisions.

The local provider store may cache metadata for performance. Local cache tables are never authoritative.

## State Model

### FS State

```text
ready
syncing
conflict
error
```

`blocked` is deferred until proposals, approvals, or hooks exist.

### Path State

Existing source/path state remains the base:

```text
ready
syncing
conflict
error
```

MVP AgentFS detail fields:

- `dirty_local`
- `last_commit_id`
- `last_committed_by`
- `base_commit_id`

### Commit State

The MVP should keep commit state simple:

```text
accepted
failed_to_materialize
```

`draft`, `proposed`, `checking`, `blocked`, and `rejected` belong to the later proposal/approval layer.

## Commit Semantics

Commit is the only governed mutation boundary in MVP.

Commit flow:

```text
collect local diff
identify committing agent
ask Section Control Service to check commit grant
check freshness against current FS head
accept or reject locally
if accepted:
  append commit record
  update FS head
  append event
  materialize accepted state to backing source
  update path sync state
  notify watchers
```

MVP commit rules:

- A commit must name the agent performing it.
- A commit must identify changed paths.
- A commit must include a summary.
- A commit must be based on the current known FS head.
- A commit can be accepted only if the committing agent has `commit`.
- Accepted commits must be observable.
- If materialization fails, the FS is not `ready` until the backing source catches up.

Conflict handling should reuse existing stale-overwrite protection.

## AGENTS.md

`AGENTS.md` remains important, but enforcement is not part of the core MVP.

MVP behavior:

- `AGENTS.md` may exist at the FS root.
- Section should preserve and sync it like any other file.
- Section may surface its presence in status output.
- Section should not claim to enforce free-form Markdown rules in MVP.

Post-MVP behavior:

- changing `AGENTS.md` requires `manage` or owner approval
- optional machine-readable policy frontmatter
- policy checks integrated into commit acceptance

## Hooks

Hooks are deferred.

Reason: hooks require a trust model. Before hooks can block commits, the design must define:

- who can install hooks
- where hooks execute
- which identity hooks run as
- whether hooks receive secrets
- whether hook output is trusted
- how hook failures affect commit state

Post-MVP hook events:

```text
fs.attach
grant.changed
commit.preflight
commit.accepted
commit.failed
conflict.detected
```

MVP should reserve the event names but not implement hook execution.

## Control Plane Surface

MVP CLI surface:

```text
section agent login
section agent identify

section fs create <name> --source-profile <profile>
section fs list
section fs grant <fs> <agent> --role reader|writer|manager
section fs revoke <fs> <agent>
section fs share <fs> <agent>
section fs available
section fs accept <share>
section fs attach <fs> <local_root>
section fs status <fs-or-local-path>

section commit status <local_path_or_root>
section commit apply <local_path_or_root> --message <text>

section watch <local_path_or_root>
```

Deferred CLI surface:

```text
section hooks add/list/remove
section commit propose/accept/reject
```

Compatibility:

- Existing `source` commands remain lower-level sync infrastructure.
- AgentFS-backed sources are managed through source profiles, not manual source setup.
- `fs create` binds an FS to a server-side source profile.
- `fs accept` records that this agent accepted a server-side share.
- `fs attach` creates the local root binding and initial sync using server-issued credentials.
- Existing `path inspect`, `path compare`, and `path resolve` remain diagnostics.

## Data Plane Layout

Attached FS directory:

```text
project/
  AGENTS.md
  docs/
  src/
  .section/
    root.json
```

`.section/root.json` remains the local discovery marker.

The backing source may mirror AgentFS metadata under `.section/agentfs/`. The local mount may hide or ignore that namespace in normal file workflows. The mirror is not the authority for grant, share, source profile, or credential decisions.

Do not materialize a large local `.section/*` control tree in MVP.

## Persistence Model

Service-owned records:

| Record | Purpose |
| --- | --- |
| `agents` | logical agent identity |
| `installations` | local runtime or machine identity |
| `source_profiles` | remote backing source selected by owner/org/server |
| `grants` | active or revoked FS permissions |
| `shares` | server-side access records for discovery and accept |
| `credential_bindings` | short-lived sync credential issuance and audit |

Backing-source metadata mirror:

| Record | Purpose |
| --- | --- |
| `fs.json` | FS id, name, owner, backing source metadata |
| `heads/current.json` | current accepted FS head |
| `grants/<grant_id>.json` | active or revoked grant records |
| `commits/<commit_id>.json` | accepted commit records |
| `events/<event_id>.json` | observable AgentFS events |
| `locks/head.json` | conservative metadata write lock |

MVP local cache tables may mirror:

- agents
- filesystems
- fs grants
- fs commits
- fs events

Existing local substrate remains:

- `sources`
- `source_local_roots`
- `path_sync_state`
- `sync_events`
- `local_scan_cache`
- `remote_manifest`

Local cache tables are not authoritative.

## Runtime Flow

### Create FS

```text
agent logs in
agent runs fs create
Section Control Service creates FS
Section Control Service binds source profile
Section Control Service records owner grant
Section Control Service appends fs.created event
```

### Share FS

```text
owner grants another agent
owner runs fs share
Section Control Service creates server-side share record
target agent can discover the share after login
```

### Accept FS

```text
target agent logs in
target agent runs fs available or fs accept
Section Control Service checks share and grant
Section Control Service returns FS metadata and source profile
Section Control Service issues short-lived sync credential
local store records accepted FS and credential binding
```

### Attach FS

```text
agent requests attach
Section Control Service checks read grant
Section Control Service issues or refreshes sync credential
Section creates local root binding
Section writes .section/root.json
Section syncs current materialized state locally
Section appends fs.attached event
```

### Commit

```text
agent edits local files
agent runs commit apply
Section collects local diff
Section checks shared head
Section Control Service checks commit grant
Section checks freshness
Section appends accepted commit
Section updates shared head
Section appends commit.accepted event
Section materializes files to backing source
Section updates local path state
```

### Observe

```text
agent runs watch
Section streams source/path events and AgentFS events
agent reacts to accepted commits, grants, and materialization errors
```

## Implementation Phases

### Phase 1: AgentFS Core Metadata

- Add AgentFS docs and terminology.
- Add service-backed agent identity.
- Add installation identity.
- Add FS metadata record.
- Add source profile record.
- Keep `.section/agentfs/` as metadata mirror, not access authority.

### Phase 2: Grants, Shares, And Attach

- Add server-side grant registry.
- Add server-side share registry.
- Add credential broker for short-lived sync credentials.
- Add `fs grant` / `fs revoke`.
- Add `fs share` / `fs available` / `fs accept`.
- Enforce read grant on `fs attach`.
- Guard AgentFS-backed sources from low-level source commands.

### Phase 3: Commit Boundary

- Add `commit status`.
- Add `commit apply`.
- Append accepted commit records.
- Update FS head.
- Materialize accepted commits through existing sync/source machinery using server-issued credentials.
- Emit AgentFS events.

### Phase 4: Stabilize Eventing

- Merge AgentFS events with existing `watch` output.
- Add diagnostics for materialization lag and errors.
- Verify multi-agent attach/commit/observe flow across two local roots.

### Phase 5: Collaboration Layer

- Add proposal/approval states if direct commit is insufficient.
- Add path-scoped grants if FS-wide roles are too coarse.

### Phase 6: Automation And Rules

- Add hook definitions and hook trust model.
- Add pre-commit hook execution.
- Add `AGENTS.md` policy enforcement only after hook/rule trust is defined.

## 后续功能具体设计

本节只定义 AgentFS 后续要补齐的能力。

原则很简单：

```text
Agent 可以拥有 FS。
Agent 可以把 FS 授权给其他 Agent。
其他 Agent 可以在许可下 attach、编辑、提交。
所有进入共享真相的修改，都必须留下可解释、可追踪、可观察的记录。
```

后续功能不改变这个模型，只把它做完整。

### 优先级

| 优先级 | 目标 | 包含功能 |
| --- | --- | --- |
| P0 | 先保证治理真相可靠 | dirty/base 判断、root 身份稳定、提交快照一致性、file/dir 替换预检、物化修复、元数据校验、事件不可变、JSON 错误、source/path 保护 |
| P1 | 再保证多 Agent 好用 | 授权后接入、事件 replay/watch、提交审计、状态命令 |
| P2 | 最后加自动化和规则 | hooks、`AGENTS.md` 规则执行、proposal/approval、path-scoped grants |

P0 和 P1 是 AgentFS 产品完整性。P2 是扩展层，不能反过来拖慢核心。

### 完整完成目标

AgentFS 核心完整完成，指它可以安全地给多个 Agent 使用。

必须同时做到：

- Agent 身份清楚。
- Installation 身份清楚。
- FS 由 Agent 拥有。
- 跨机分享必须走 Section Control Service。
- SourceProfile 由服务端决定。
- sync credential 由服务端短期发放。
- 被授权 Agent 可以发现、accept、attach。
- reader 只能读，不能提交。
- writer 可以提交。
- manager 可以管理 grant 和 share，但不能自动获得提交权。
- 所有进入共享真相的修改都必须走 commit。
- accepted commit 是治理真相。
- backing source 是物化后的文件状态。
- 本地目录只是 working copy。
- commit 必须检查权限、base、dirty、reserved path。
- commit 必须先冻结 staging snapshot，再写 metadata，再物化文件。
- accepted commit 必须记录谁提交、哪个 grant 或 owner authority 允许。
- materialization 失败时，head 不回滚，后续 commit 被阻塞。
- repair 必须修复同一个 commit，不能制造第二个 truth。
- Agent 可以用 `fs status` 判断自己现在能不能行动。
- Agent 可以用 events/watch 看到 FS 发生了什么。
- JSON 错误必须有稳定 `code`、`retryable`、`details`。
- 低层 `source/path` 命令不能绕过 AgentFS 治理。
- 代码、测试、设计文档必须一致。

不算完整完成：

- 只能在 owner 本机配置好以后使用。
- 要靠手动传 source 密钥分享。
- 要靠读 `.section/agentfs` metadata 才知道发生了什么。
- commit metadata 和实际物化文件可能不是同一份内容。
- materialization 失败后只能人工清理。
- 低层 source 命令可以绕过 AgentFS。
- 文档说支持，但 CLI 或 E2E 没有证明。

### 1. 授权后接入

现在的问题：

```text
owner grant 了 writer，但 writer 需要通过服务端发现、接受、获取同步凭据。
```

目标：

```text
grant 之后，被授权 Agent 登录服务端就能发现 FS、accept、attach。
```

设计：

- owner 或 manager 执行 `section fs share <fs> <agent_id> --role writer`。
- Section Control Service 创建 server-side share record。
- share record 指向 `fs_id`、`grant_id`、`target_agent_id`、`source_profile_id`。
- 被授权 Agent 登录后执行 `section fs available`。
- 被授权 Agent 执行 `section fs accept <share_id>`。
- accept 时服务端校验 agent 身份、share 状态、grant 状态。
- accept 成功后，服务端返回 FS metadata、source profile、短期 sync credential。
- 本地 store 只保存 accepted FS、mount state、credential binding，不保存 source 长期密钥。

元数据：

```text
Section Control Service:
  shares/<share_id>
  grants/<grant_id>
  source_profiles/<source_profile_id>
  credential_bindings/<binding_id>
```

share 字段：

- `share_id`
- `fs_id`
- `target_agent_id`
- `grant_id`
- `role`
- `source_profile_id`
- `created_by`
- `created_at_ms`
- `expires_at_ms`
- `accepted_at_ms`
- `revoked_at_ms`

规则：

- 只有 owner 或有 `manage` 的 Agent 能创建 share。
- share 只能被目标 `agent_id` 接受。
- share 过期或 revoked 后不能 accept。
- accept 必须走 Section Control Service。
- share 不包含 source 长期密钥。
- sync credential 由服务端按 grant 发放，必须短期有效。
- 如果 grant 被 revoke，后续 attach、credential refresh、commit 都会失败。

验收：

- writer 登录后能在 `fs available` 看到共享给自己的 FS。
- writer accept 后能 attach。
- reader accept 后只能 read，不能 commit。
- revoked share 不能再被 accept。
- share 和 accept 过程中不会暴露 source 长期密钥。

### 2. AgentFS 事件 replay 和 watch

现在的问题：

```text
测试可以直接读 metadata 文件，但真实 Agent 需要命令级事件流。
```

目标：

```text
Agent 能看到自己关心的 FS 发生了什么。
```

设计：

- 增加 `section fs events <fs-or-root> [--after <event_id>] [--limit N] [--json]`。
- 增加 `section watch <fs-or-root> --agentfs`，持续输出 AgentFS 事件。
- `watch` 可以合并 source/path 事件，但每条输出必须带 `stream` 字段。
- AgentFS 事件必须有单调 `seq`，不能只依赖时间戳排序。

事件输出：

```json
{
  "stream": "agentfs",
  "seq": 42,
  "event_id": "evt_...",
  "fs_id": "fs_...",
  "kind": "commit.accepted",
  "actor_agent_id": "agt_...",
  "subject_id": "cmt_...",
  "created_at_ms": 1780000000000,
  "data": {}
}
```

元数据：

- event record 增加 `seq`。
- `seq` 在 head lock 内分配。
- replay 默认按 `seq` 排序。

验收：

- observer 可以通过命令看到 `commit.accepted`。
- observer 可以从某个 `event_id` 或 `seq` 后继续 replay。
- 同一个 FS 内事件顺序稳定。

### 3. 提交审计

现在的问题：

```text
commit 记录了谁提交，但没有记录这次提交是由哪个 grant 或 owner authority 允许的。
```

目标：

```text
任何 accepted commit 都能解释：谁做的，为什么被允许。
```

设计：

commit record 增加：

```json
{
  "authorized_by": {
    "type": "grant",
    "grant_id": "grt_...",
    "role": "writer",
    "capabilities": ["read", "commit"]
  }
}
```

owner 自己提交时：

```json
{
  "authorized_by": {
    "type": "owner",
    "agent_id": "agt_..."
  }
}
```

规则：

- commit accept 时必须重新读取当前 grant。
- 记录当时生效的 grant id。
- grant 之后被 revoke，不影响旧 commit 的审计解释。
- `commit.accepted` event 的 `data` 里也带 `authorized_by` 摘要。

验收：

- writer commit 后，commit record 能指向 grant id。
- grant revoke 后，旧 commit 审计信息仍然可读。

### 4. 可信 dirty/base 判断

现在的问题：

```text
只看 .section/root.json 不够可信，也不够表达本地 working copy 的真实 base。
```

目标：

```text
commit status/apply 知道本地改动是基于哪个 accepted head 做出来的。
```

设计：

- `.section/root.json` 只作为本地发现入口。
- 权威 mount 状态写入本地 provider store。
- mount 状态记录 `mount_id`、`fs_id`、`canonical_root`、`base_commit_id`、`base_manifest_hash`、`last_sync_at_ms`。
- `commit status` 比较本地 tree 和 `base_manifest`，得到 dirty paths。
- `commit apply` 同时检查本地 `base_commit_id` 和 shared `heads/current.json`。
- 如果 shared head 已经前进，返回 `stale_base`。

验收：

- 同一个 root 用不同 path spelling 调用，也能找到同一个 mount。
- stale writer 不能覆盖新 head。
- status 能区分 clean、dirty、stale、materialization error。

### 5. 提交快照一致性

现在的问题：

```text
commit metadata 可能描述了版本 A，但物化时本地文件已经变成版本 B。
```

目标：

```text
accepted commit metadata 描述的内容，必须就是物化到 backing source 的内容。
```

设计：

- `commit apply` 先把 dirty paths 冻结到本地 staging snapshot。
- path hash 从 staging snapshot 计算。
- commit record 写入的 path version 来自 staging snapshot。
- materialization 只能从 staging snapshot 读，不再读 live working tree。
- 如果 staging 失败，commit 在写 metadata 前失败。

本地 staging 路径由实现决定，例如：

```text
<local-data-dir>/agentfs/staging/<fs_id>/<commit_id>/
```

规则：

- staging snapshot 是 commit 的输入。
- live working tree 在 commit 过程中继续变化，不影响本次 commit。
- materialized 后可以 GC staging，但要保留到 repair 不再需要。

验收：

- 测试在 commit 过程中修改文件，accepted commit 仍然和物化内容一致。

### 6. 物化失败和修复

现在的问题：

```text
accepted commit 可以 failed_to_materialize，但缺少明确 repair 命令。
```

目标：

```text
失败的 accepted commit 可以用同一个 commit_id 修复，不产生第二个 shared truth。
```

设计：

- 增加 `section commit repair <fs-or-root> [--commit <commit_id>]`。
- repair 只处理 `pending` 或 `failed_to_materialize` 的 commit。
- repair 使用原 commit 的 staging snapshot。
- 如果 staging snapshot 不存在，返回 `missing_commit_snapshot`。
- MVP 不提供 live root 兜底 repair。

规则：

- repair 不创建新 commit。
- repair 成功后更新 commit 的 `materialization_state` 为 `materialized`。
- repair 成功后写 `commit.materialized` event。
- 当前 head 未物化时，新的 `commit apply` 继续被阻塞。

验收：

- failed commit repair 后，head 仍然是同一个 commit id。
- repair 成功前，后续 commit 被拒绝。

### 7. 本地 root 身份

现在的问题：

```text
同一个目录可能有不同 path spelling，父子目录也可能形成嵌套 mount。
```

目标：

```text
Section 稳定识别每个 attached root，避免一个 FS 的控制信息被另一个 FS 当成普通文件。
```

设计：

- attach 前 canonicalize local root。
- `mount_id = hash(fs_id + canonical_root)`。
- 本地 store 以 `mount_id` 记录 mount。
- 同一个 Agent 可以把同一个 FS attach 到多个不重叠 root。
- 禁止同一个 data dir 下出现父子重叠 mount。
- 禁止把 backing source root 直接作为 working root。

验收：

- `/tmp/a/../a` 和 `/tmp/a` 解析到同一个 mount。
- attach parent/child root 会失败。
- commit/status 不会误读别的 root marker。

### 8. source/path 命令保护

现在的问题：

```text
source/path 是底层同步基础设施。直接操作它们可能绕过 AgentFS 治理。
```

目标：

```text
低层命令可以保留，但默认不能破坏 AgentFS。
```

设计：

- source record 增加 `owned_by_agentfs: true` 或等价标记。
- `source sync/remove/bind/unbind` 遇到 AgentFS-backed source 时拒绝。
- MVP 不提供低层 `source` 命令的强制绕过参数。
- 如果以后需要维修，单独设计 `section admin` 或 `section fs repair` 命令，并记录审计。
- `path resolve/inspect/compare` 保持诊断用途，但输出 warning：这是底层视图，不代表 AgentFS 治理结果。
- `.section/agentfs/**` 永远不能作为普通用户路径 sync 或 commit。

验收：

- 普通 `source sync` 不能静默改变 AgentFS materialized state。
- 普通 `source` 命令不能绕过 AgentFS 治理。

### 9. 统一 JSON 错误

现在的问题：

```text
不同命令可能返回不同形状的错误，不利于 Agent 自动处理。
```

目标：

```text
Agent 可以稳定解析错误 code，并判断能不能重试。
```

设计：

所有 AgentFS 命令在 `--json` 下返回：

```json
{
  "error": {
    "code": "stale_base",
    "message": "local base is behind current fs head",
    "retryable": true,
    "details": {}
  }
}
```

规则：

- `code` 稳定，不能随文案变化。
- `message` 给人看。
- `retryable` 给 Agent 决策。
- `details` 放结构化上下文。
- 未分类内部错误也要包成 `internal_error`。

验收：

- 每个公开 AgentFS 错误至少有一个 JSON 测试。

### 10. 元数据校验和坏数据隔离

现在的问题：

```text
共享 metadata 是治理真相，坏 metadata 不能随便污染所有命令。
```

目标：

```text
读 metadata 时严格校验。坏记录只影响相关 FS 或相关候选，不影响无关 FS。
```

设计：

- 为 `fs`、`grant`、`commit`、`event`、`head` 增加统一 validate。
- 校验 `schema_version`、id 格式、`fs_id` 一致性、path 格式、timestamp、枚举值。
- 校验 head 指向的 commit 存在。
- 校验 commit parent 和 head 关系。
- 校验 active grant 不出现重复冲突。
- 增加 `section fs validate <fs-or-root>` 做诊断。

查找规则：

- 精确 `fs_id` 优先。
- 本地 attached root 第二。
- 精确 source name 第三。
- 精确 FS name 第四。
- 多个候选命中时返回 `ambiguous_fs_ref`。
- 非候选 source 的坏 metadata 只作为 warning。

验收：

- 一个 source 有坏 metadata，不影响另一个正常 FS 的 status。
- ambiguous name 返回明确错误。

### 11. 事件不可变和排序

现在的问题：

```text
事件文件如果可覆盖，审计流就不可信。
```

目标：

```text
事件一旦写入就不可变，同一个 FS 内有稳定顺序。
```

设计：

- event 写入使用 create-if-absent 能力。
- backend 不支持 create-if-absent 时，在 head lock 内先检查不存在再写。
- event record 增加 `seq`。
- `seq` 在 head lock 内从 `events/state.json` 或 head metadata 中递增。
- 读取事件时按 `seq` 排序。

规则：

- 事件不能被 update。
- 需要表达状态变化时写新事件。
- repair 也写新事件，不改旧事件。

验收：

- 重复写同一个 `event_id` 失败。
- replay 顺序不依赖文件系统 list 顺序。

### 12. status 作为 Agent 的主要入口

现在的问题：

```text
Agent 需要一个命令判断自己现在能做什么。
```

目标：

```text
section fs status 告诉 Agent：我是谁，我在看哪个 FS，我有没有权限，当前能不能提交。
```

设计：

`section fs status <fs-or-root> --json` 输出：

```json
{
  "fs_id": "fs_...",
  "name": "project",
  "agent_id": "agt_...",
  "role": "writer",
  "capabilities": ["read", "commit"],
  "head_commit_id": "cmt_...",
  "base_commit_id": "cmt_...",
  "dirty": true,
  "dirty_count": 2,
  "stale": false,
  "materialization_state": "materialized",
  "last_event_seq": 42,
  "warnings": []
}
```

规则：

- 参数可以是 fs id、fs name、source name、local path。
- local path 优先解析 attached root。
- status 不修改共享状态。
- status 可以返回 warning，但不能把 warning 当成功治理结果。

验收：

- Agent 不读 metadata 文件，也能知道是否能 commit。

### 13. file/dir 替换预检

现在的问题：

```text
file -> dir 或 dir -> file 的替换，如果物化顺序错了，会留下错误状态。
```

目标：

```text
commit 在接受前就知道物化计划能不能安全执行。
```

设计：

- `commit apply` 生成 path operation plan。
- file -> dir：先 delete file，再 create dir，再 create children。
- dir -> file：先 delete subtree，按 deepest-first 删除，再 create file。
- 如果 provider 不支持所需操作，commit 在 metadata 写入前失败。

验收：

- file/dir 互换不会留下半物化状态。
- 不支持的替换返回可解释错误。

### 14. hooks

hooks 是自动化层，不是治理核心。

目标：

```text
让 owner/manager 可以把 AgentFS 事件交给 agent 或 script 处理。
```

设计：

- 增加 `section hooks add/list/remove/test`。
- hook metadata 放在 `.section/agentfs/hooks/<hook_id>.json`。
- 只有 owner 或有 `manage` 的 Agent 可以安装、修改、删除 hook。
- hook 默认是 post-event automation，不阻塞 commit。
- `commit.preflight` hook 可以阻塞 commit，但必须显式声明 `blocking: true`。

hook record：

```json
{
  "schema_version": 1,
  "hook_id": "hook_...",
  "fs_id": "fs_...",
  "event": "commit.accepted",
  "command": ["section-agent", "handle-commit"],
  "blocking": false,
  "timeout_ms": 30000,
  "installed_by": "agt_...",
  "created_at_ms": 1780000000000
}
```

规则：

- hook 默认不拿 secrets。
- hook 输入是结构化 JSON event。
- hook 输出也必须是 JSON。
- blocking hook 运行在提交方当前 installation。
- 服务端只保存 hook 配置、事件和审计，不负责通用脚本执行。
- blocking hook timeout 或失败时，commit 返回明确错误。
- 非 blocking hook 失败只写 `hook.failed` event，不回滚 commit。

验收：

- post hook 能收到 `commit.accepted`。
- blocking preflight hook 能阻止 commit。
- 没有 manage 权限不能安装 hook。

### 15. `AGENTS.md` 规则

`AGENTS.md` 是 FS 本地规则，不是普通说明文档。

目标：

```text
Agent attach 或 commit 前能知道这个 FS 的工作规则。
```

设计：

- MVP 继续把 `AGENTS.md` 当普通文件同步。
- 后续执行规则时，先支持一个很小的机器可读块。
- 自然语言部分继续给 Agent 阅读，不做强制解释。

示例：

```markdown
---
section:
  required_checks:
    - cargo test
  protected_paths:
    - docs/AGENTFS_MVP_CONTRACT.md
---

# AGENTS.md

Before committing, read the product contract.
```

规则：

- 修改 `AGENTS.md` 需要 `manage` 或 owner。
- `required_checks` 可以映射到 blocking hook。
- `protected_paths` 初期只要求 `manage` 权限，不做复杂审批。
- 解析不了机器块时，普通 commit 返回 `agent_rules_invalid`。
- owner 或有 `manage` 的 Agent 可以提交修复后的 `AGENTS.md`。

验收：

- writer 修改普通文件可以 commit。
- writer 修改 protected path 会被拒绝。
- manager 修改 `AGENTS.md` 可以通过。
- invalid `AGENTS.md` 不会静默关闭已有规则。

### 16. path-scoped grants

path-scoped grants 是 FS-wide grants 的扩展，不是第一版核心。

目标：

```text
owner 可以只允许某个 Agent 提交某些路径。
```

设计：

- grant record 增加可选 `path_scopes`。
- 没有 `path_scopes` 时，保持 FS-wide 行为。
- 第一版只做 allow scopes，不做 deny scopes。
- `commit apply` 要求本次 commit 的所有 changed paths 都落在允许范围内。
- owner 不受 path scope 限制。
- manager 只能管理 grant，不能因为管理了某个 path scope 就获得 commit 权限。

grant 示例：

```json
{
  "grant_id": "grt_...",
  "fs_id": "fs_...",
  "agent_id": "agt_...",
  "role": "writer",
  "capabilities": ["read", "commit"],
  "path_scopes": ["docs/**", "README.md"]
}
```

规则：

- path scope 使用 FS-root-relative glob。
- scope 不能包含绝对路径、`..`、空 segment、`.section/agentfs/**`。
- 一个 commit 里只要有一个路径越界，整个 commit 被拒绝。
- `authorized_by` 要记录 grant id 和命中的 path scopes。

验收：

- writer 修改 `docs/a.md` 可以 commit。
- writer 修改 `src/a.rs` 会被拒绝。
- 同一个 commit 同时包含允许路径和越界路径，也会被拒绝。

### 17. proposal/approval

proposal/approval 不是当前核心路径。

只有当直接 commit 不够用时再加。

设计方向：

- 增加 `propose` capability。
- 增加 `contributor` role：`read` + `propose`。
- `section commit propose <root> --message <text>` 创建 proposal。
- proposal 使用同样的 staging snapshot。
- owner 执行 `section commit accept <proposal_id>`。
- 非 owner 接受 proposal 必须同时具备 `manage` 和 `commit`。
- accept 后 proposal 变成 accepted commit。
- reject 需要 `manage`，只关闭 proposal，不改变 shared truth。

规则：

- proposal 不推进 head。
- accepted proposal 必须重新检查 freshness。
- proposal 过期后不能 accept。
- 只有 `manage` 但没有 `commit` 的 manager 不能 accept proposal。

验收：

- contributor 没有直接 commit 权限时，可以 propose。
- owner accept 后，FS head 前进。
- manager-only accept 会被拒绝。

## 已定业务决策

这些问题已经按 MVP 固定下来。

### SourceProfile

- `fs create` 可以传一个 source profile 请求。
- 最终绑定哪个 SourceProfile，由 Section Control Service 决定。
- 本地 CLI 不创建跨机分享用的 source profile。
- 被分享的 Agent 不能自己选择远程 source。

### Credential

- sync credential 只短期有效。
- credential 由 Section Control Service 按 grant 发放。
- grant revoke 后，不再发新 credential。
- 已发 credential 到期后自然失效。
- CLI 输出和 share record 都不能包含 source 长期密钥。

### Materialization

- MVP 由提交方本地 installation 执行 materialization。
- 提交方只能使用服务端发的短期 write credential。
- 服务端物化不是 MVP 路径。
- 如果本地物化失败，commit 仍然 accepted，FS 进入需要 repair 的状态。

### Hooks

- MVP hooks 是后续功能，不进入当前核心。
- blocking hook 如果实现，运行在提交方当前 installation。
- Section Control Service 只保存 hook 配置和审计，不提供通用执行沙箱。
- 需要远程 runner 时，另做产品设计。

### Owner

- MVP 只有 Agent 可以成为 FS owner。
- 人类用户可以管理 Agent identity，但不直接成为 FS owner。
- 以后如果需要人类 owner，要单独设计组织、账号、审计，不混进 AgentFS MVP。

## Design Principle

Keep the first version narrow.

```text
Build one excellent agent-governed filesystem.
Use existing source/path sync as substrate.
Make commit the governance boundary.
Make accepted commits the governance truth.
Make backing source state the materialized truth.
Make every accepted mutation observable.
```
