# Section AgentFS MVP Contract

## Purpose

This document answers the Phase 1 readiness questions in [AGENTFS_DEVELOPMENT_READINESS.md](AGENTFS_DEVELOPMENT_READINESS.md).

It is the development contract for the first implementation slice:

```text
Agent / Installation / FS / SourceProfile / Grant / Share / Credential / Commit / Event
```

Hooks, proposal/approval workflows, path-scoped grants, and `AGENTS.md` enforcement are out of scope for this contract.

正式产品的跨机 / 多 agent sharing 依赖 Section Control Service。服务端是 agent identity、installation、grant、share、source profile、credential 的权威。

## 1. Terms And Identity

IDs are stable, opaque strings. Display names are metadata and can change.

| Term | Format | Notes |
| --- | --- | --- |
| `agent_id` | `agt_` + 32 lowercase hex chars | Issued by Section Control Service and cached locally |
| `installation_id` | `ins_` + 32 lowercase hex chars | One local machine/runtime for an agent |
| `fs_id` | `fs_` + 32 lowercase hex chars | Generated at FS creation; independent from source profile name |
| `source_profile_id` | `srcp_` + 32 lowercase hex chars | Server-side backing source profile |
| `share_id` | `shr_` + 32 lowercase hex chars | Server-side share record |
| `credential_binding_id` | `cred_` + 32 lowercase hex chars | Short-lived sync credential audit record |
| `mount_id` | derived from `fs_id + canonical_local_root` in MVP | No persistent mount id required in the first implementation |
| `commit_id` | `cmt_` + 32 lowercase hex chars | Generated before commit metadata is written |
| `event_id` | `evt_` + 13 digit epoch-ms + `_` + 16 lowercase hex chars | Event identity only; ordering uses `seq` |
| `event_seq` | per-FS integer, starts at 1 | Strictly increases inside one FS |
| `grant_id` | `grt_` + 32 lowercase hex chars | Generated when a grant is created |

All timestamps are Unix epoch milliseconds in UTC stored as integers.

Path strings are UTF-8, slash-separated, source-root-relative paths:

- no leading slash
- no `..` segments
- no empty segment except the root path represented as `""`
- `.section/agentfs/**` is reserved control metadata

## 2. Authority Model

The truth hierarchy is:

```text
Section Control Service = identity, grant, share, source profile, credential authority
accepted commit log = governance truth
backing source = materialized filesystem state
local mount = working copy
```

A commit is accepted when:

1. Section Control Service confirms the committing agent has an active `commit` capability,
2. the local base commit equals the current shared head,
3. the commit does not include reserved metadata paths,
4. the commit record is written under `.section/agentfs/commits/`,
5. `heads/current.json` points to that commit.

The backing source can lag behind the accepted commit log.

If commit metadata succeeds but file materialization fails:

- the commit remains the governance truth,
- `heads/current.json` remains pointed at the accepted commit,
- commit materialization state becomes `failed_to_materialize`,
- FS state is `error`,
- further commits are blocked until materialization is repaired.

Watchers can rely on accepted commit records and AgentFS events as governance authority. File reads rely on the materialized backing source and may lag while FS state is `syncing` or `error`.

External edits to the backing source are not governed commits. They are detected by source/path comparison and should surface as drift or conflict, using the existing stale-overwrite protection.

## 3. Control Metadata Layout

Service-owned records:

```text
agents
installations
source_profiles
grants
shares
credential_bindings
```

AgentFS metadata may be mirrored under the backing source:

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

Rules:

- `.section/agentfs/**` is reserved.
- Normal commits cannot modify reserved metadata paths.
- The local mount may expose `.section/root.json` for discovery.
- The local mount should not expose `.section/agentfs/**` as normal user content when it can avoid it.
- If reserved metadata appears in a working copy, `commit apply` must ignore it for dirty detection and reject explicit attempts to commit it.

Metadata files are rewritten as whole JSON documents. The MVP does not use JSONL append files because object backends do not provide portable append semantics.

Authority rules:

- Section Control Service is authoritative for identity, grant, share, source profile, and credential decisions.
- The backing-source metadata namespace is a mirror for materialization, audit, repair, and diagnosis.
- Normal commits cannot modify mirrored metadata paths.
- `fs list`, `fs status`, `fs events`, and `watch --agentfs` only expose FS metadata to agents with `read` capability.

### Metadata Write Lock

The MVP uses a single FS head lock:

```text
.section/agentfs/locks/head.json
```

Lock fields:

```json
{
  "schema_version": 1,
  "fs_id": "fs_...",
  "lock_token": "lck_...",
  "owner_agent_id": "agt_...",
  "created_at_ms": 1780000000000,
  "expires_at_ms": 1780000030000
}
```

Rules:

- The lock protects head updates and commit acceptance.
- Grant, share, source profile, and credential changes are protected by Section Control Service.
- A stale lock can be replaced after `expires_at_ms`.
- If the lock cannot be acquired, commands return `metadata_write_conflict`.
- The first implementation can keep lock semantics conservative and fail closed.

## 4. Metadata Schemas

All records include `schema_version: 1`.
Readers must reject shared metadata with the wrong schema version, invalid IDs,
invalid AgentFS paths, inconsistent grant capabilities, or cross-record links
that point at another FS. The JSON error code is `malformed_shared_metadata`.

### `fs.json`

```json
{
  "schema_version": 1,
  "fs_id": "fs_...",
  "name": "project",
  "owner_agent_id": "agt_...",
  "source_profile_id": "srcp_...",
  "created_at_ms": 1780000000000
}
```

### Grant Record

```json
{
  "schema_version": 1,
  "grant_id": "grt_...",
  "fs_id": "fs_...",
  "agent_id": "agt_...",
  "role": "writer",
  "capabilities": ["read", "commit"],
  "granted_by": "agt_...",
  "created_at_ms": 1780000000000,
  "revoked_at_ms": null,
  "revoked_by": null
}
```

`revoked_at_ms != null` makes the grant inactive.

### Commit Record

```json
{
  "schema_version": 1,
  "commit_id": "cmt_...",
  "fs_id": "fs_...",
  "parent_commit_id": "cmt_...",
  "base_commit_id": "cmt_...",
  "base_manifest_hash": "sha256:...",
  "agent_id": "agt_...",
  "summary": "Update docs",
  "authorized_by": {
    "type": "grant",
    "grant_id": "grt_...",
    "role": "writer",
    "capabilities": ["read", "commit"]
  },
  "paths": [
    {
      "path": "docs/readme.md",
      "kind": "file",
      "op": "update",
      "local_version": "sha256:...",
      "previous_version": "sha256:..."
    }
  ],
  "staging_snapshot": {
    "manifest_path": "agentfs/staging/fs_.../cmt_.../manifest.json",
    "manifest_hash": "sha256:..."
  },
  "created_at_ms": 1780000000000,
  "materialization_state": "pending",
  "materialized_at_ms": null,
  "error": null
}
```

Allowed `op` values:

- `create`
- `update`
- `delete`

Allowed `materialization_state` values:

- `pending`
- `materialized`
- `failed_to_materialize`

`authorized_by.type` is either `owner` or `grant`.

Owner commit:

```json
{
  "type": "owner",
  "agent_id": "agt_..."
}
```

Grant commit:

```json
{
  "type": "grant",
  "grant_id": "grt_...",
  "role": "writer",
  "capabilities": ["read", "commit"]
}
```

The commit record keeps this audit even if the grant is revoked later.

### `heads/current.json`

```json
{
  "schema_version": 1,
  "fs_id": "fs_...",
  "commit_id": "cmt_...",
  "updated_at_ms": 1780000000000
}
```

For a newly created empty FS, `commit_id` is `null`.

### Event Record

```json
{
  "schema_version": 1,
  "event_id": "evt_1780000000000_0123456789abcdef",
  "seq": 42,
  "fs_id": "fs_...",
  "kind": "commit.accepted",
  "actor_agent_id": "agt_...",
  "subject_id": "cmt_...",
  "path": null,
  "created_at_ms": 1780000000000,
  "data": {}
}
```

Event `path` is optional. FS-level events use `null`.

## 5. Grant Semantics

MVP roles are FS-wide.

| Role | Capabilities |
| --- | --- |
| `owner` | `read`, `commit`, `manage` |
| `reader` | `read` |
| `writer` | `read`, `commit` |
| `manager` | `read`, `manage` |

Rules:

- The creating agent receives an owner grant.
- Owner cannot be revoked.
- Ownership transfer is deferred.
- `manager` cannot grant `owner`.
- `manager` can create, revoke, or replace `reader`, `writer`, and `manager` grants.
- Revoked agents cannot attach or commit after revocation is observed.
- Existing raw local files remain on disk after revocation, but they cannot become shared truth through AgentFS commit.

## 6. Commit Semantics

`commit apply` commits all dirty paths under the attached root in MVP.

Partial path commits are deferred.

Dirty paths are discovered by comparing the local tree against the last known mounted base state and current path sync state.

Freshness base:

- the trusted local mount store records `base_commit_id` for the attached working copy.
- `.section/root.json` helps discover the mount identity, but it is not trusted for freshness.
- `heads/current.json` records current shared head.
- commit is rejected with `stale_base` when the trusted local mount base and current shared head differ.

Rules:

- summary is required and must be non-empty after trimming whitespace.
- empty commits are rejected.
- commits containing `.section/agentfs/**` are rejected.
- reserved metadata paths are ignored in dirty detection.
- external backing-source drift is treated as conflict or stale state before commit acceptance.

提交输入规则：

- `commit apply` 先把本次 dirty paths 复制到本地 staging snapshot。
- `paths[*].local_version` 从 staging snapshot 计算。
- commit record 里的 paths 必须来自 staging manifest。
- materialization 只能读取 staging snapshot，不能再读 live working tree。
- 如果 staging snapshot 创建失败，commit 不写 metadata。
- 如果 live working tree 在 commit 过程中继续变化，这些变化属于下一次 dirty work。

## 7. Materialization Semantics

Commit acceptance writes governance metadata first:

1. create staging snapshot,
2. acquire head lock,
3. verify grant and freshness,
4. write commit record with `materialization_state: "pending"`,
5. update `heads/current.json`,
6. write `commit.accepted` event,
7. release head lock,
8. materialize file changes from staging snapshot to backing source,
9. update commit materialization state,
10. write `commit.materialized`, or write both `commit.materialization_failed` and `fs.error`,
11. update local mount store and root marker as a local finalization step.

If step 11 fails after accepted metadata and backing-source materialization have
succeeded, the command still succeeds and returns a local warning. The accepted
commit remains the governance truth.

If materialization fails:

- commit remains accepted,
- FS state becomes `error`,
- `commit apply` is blocked for that FS,
- attach reports materialization error,
- repair is required before further accepted commits.

Retry behavior:

- retry is safe only for commits in `pending` or `failed_to_materialize`.
- retry must use the existing `commit_id`.
- retry must not create a second accepted commit.

## 8. Event Semantics

MVP event kinds:

```text
fs.created
fs.attached
grant.created
grant.revoked
commit.accepted
commit.materialized
commit.materialization_failed
fs.error
```

Event records are immutable.

Replay:

- event files are listed under `.section/agentfs/events/`,
- clients sort by `seq`,
- clients can resume from the last seen `seq` or `event_id`.

Ordering:

- `seq` is allocated while holding the FS head lock,
- events emitted under the head lock are ordered relative to commit/grant head mutations.
- materialization events happen after the accepted commit event.
- source/path events and AgentFS events may be merged in `watch`, but each output event must include a stream or kind that identifies its origin.

## 9. Local Mount Contract

`fs attach` creates a local working copy.

Attach behavior:

- checks `read` capability with Section Control Service,
- obtains or refreshes a short-lived sync credential,
- creates or updates the source local-root binding,
- writes `.section/root.json`,
- syncs current materialized backing-source state into the local root,
- records `base_commit_id` in the trusted local mount store and mirrors it into `.section/root.json`.

`root.json` fields:

```json
{
  "schema_version": 1,
  "fs_id": "fs_...",
  "source_profile_id": "srcp_...",
  "agent_id": "agt_...",
  "installation_id": "ins_...",
  "local_root": "/abs/path/project",
  "base_commit_id": "cmt_...",
  "control_plane_endpoint": "section-control-service"
}
```

Rules:

- multiple local roots may attach to the same FS if their paths are distinct.
- reader mounts can be edited locally by the OS, but reader commits are denied.
- attach should fail or report non-ready state when current head is not materialized.

## 10. SourceProfile Compatibility

MVP FS binds exactly one server-side SourceProfile.

Rules:

- `fs create` asks Section Control Service to bind a SourceProfile.
- Section Control Service makes the final SourceProfile decision.
- upgrading an existing source into an FS is deferred.
- low-level `source` commands remain sync infrastructure, not the AgentFS product surface.
- AgentFS-backed sources must be guarded from ordinary source mutation commands.
- `source sync` must not become the official way to bypass AgentFS governance.
- AgentFS commands are the governance surface; source commands are infrastructure.
- MVP does not provide a low-level force flag to mutate AgentFS-backed sources.

Documentation must not imply that low-level source commands preserve AgentFS governance.

## 11. Error Model

Errors must have stable codes for JSON output.

| Code | Meaning | Retry |
| --- | --- | --- |
| `unknown_agent` | Agent identity is missing or not logged in | no, login first |
| `unknown_fs` | FS metadata cannot be found | no, check name/id |
| `grant_denied` | Agent lacks required capability | no, request grant |
| `stale_base` | local `base_commit_id` differs from current head | yes, sync/refresh first |
| `reserved_metadata_path` | operation attempts to commit `.section/agentfs/**` | no |
| `materialization_failed` | accepted commit could not update backing source | yes, repair/retry materialization |
| `malformed_shared_metadata` | shared metadata failed schema validation | no, repair metadata |
| `metadata_write_conflict` | lock or head update conflict | yes |

JSON shape:

```json
{
  "error": {
    "code": "grant_denied",
    "message": "agent agt_... does not have commit access to fs fs_...",
    "retryable": false,
    "details": {
      "fs_id": "fs_...",
      "agent_id": "agt_..."
    }
  }
}
```

Rules:

- `code` is stable.
- `message` is for humans.
- `retryable` is for Agent decisions.
- `details` is always present in JSON output. It can be `{}`.

## 12. 状态输出合同

`section fs status <fs-or-root> --json` 是 Agent 判断能不能行动的主要入口。

输出：

```json
{
  "fs": {
    "fs_id": "fs_...",
    "name": "project",
    "head_commit_id": "cmt_...",
    "materialization_state": "materialized"
  },
  "agent": {
    "agent_id": "agt_...",
    "role": "writer",
    "capabilities": ["read", "commit"]
  },
  "mount": {
    "attached": true,
    "mount_id": "hash(fs_id + canonical_local_root)",
    "local_root": "/abs/path/project",
    "base_commit_id": "cmt_...",
    "base_manifest_hash": "sha256:..."
  },
  "worktree": {
    "dirty": true,
    "dirty_count": 2,
    "stale": false
  },
  "events": {
    "last_seq": 42
  },
  "warnings": [],
  "next_actions": ["commit"]
}
```

规则：

- `status` 可以解析 FS id、FS name、source name、local root。
- 非本地路径 ref 的解析顺序是：精确 FS id，精确 source name，精确 FS name。
- 如果 source name 或 FS name 命中多个候选，返回 `ambiguous_fs_ref`。
- local root 解析必须看本地 mount store，不能只看 `.section/root.json`。
- `status` 不修改共享状态。
- 如果 Agent 现在不能行动，`next_actions` 说明下一步安全动作，例如 `login`、`accept`、`attach`、`sync`、`repair`、`request_grant`。

## 13. 提交快照和修复合同

staging snapshot 路径：

```text
<local-data-dir>/agentfs/staging/<fs_id>/<commit_id>/
  manifest.json
  files/<hash-or-safe-path>
```

`manifest.json` 内容：

```json
{
  "schema_version": 1,
  "fs_id": "fs_...",
  "commit_id": "cmt_...",
  "base_commit_id": "cmt_...",
  "created_at_ms": 1780000000000,
  "paths": [
    {
      "path": "docs/readme.md",
      "op": "update",
      "kind": "file",
      "hash": "sha256:...",
      "size": 123,
      "staged_path": "files/sha256-..."
    }
  ]
}
```

规则：

- `commit apply` 写 commit metadata 前先创建 staging snapshot。
- commit metadata 记录 staging manifest hash。
- materialization 只能读 staging snapshot。
- staging 至少保留到 commit 已物化，并且 repair 不再需要它。

`section commit repair <fs-or-root> [--commit <commit_id>]` 规则：

- repair 只处理 `pending` 或 `failed_to_materialize` commit。
- repair 使用原来的 staging snapshot。
- repair 不创建新 commit。
- repair 不移动 head。
- repair 成功写 `commit.materialized`。
- repair 失败写 `commit.materialization_failed` 和 `fs.error`。
- 如果 staging snapshot 不存在，返回 `missing_commit_snapshot`。

MVP 不提供兜底修复路径。

## 14. 服务端合同

测试里的 file-backed Control Service 只是本地 harness。

正式产品里，服务端负责：

- agent login
- installation registration
- FS create/list/status metadata
- grant create/revoke/check
- share create/list/accept/revoke
- source profile selection
- short-lived credential issuance
- AgentFS event replay
- audit records

API 分组：

```text
/agents/login
/installations
/filesystems
/grants
/shares
/credentials
/events
```

规则：

- 跨机 sharing 必须走服务端。
- SourceProfile 由服务端决定。
- source 长期密钥不能出现在 share record、local root marker、CLI JSON 输出里。
- 本地 CLI 可以缓存 identity、accepted FS、mount state、短期 credential binding。
- 本地 CLI 不能自己发明 grant、share、source profile、长期 credential。
- 服务端拒绝时，本地 CLI 必须失败关闭。

## 15. Test Contract

The complete implementation test plan lives in
[AGENTFS_TEST_PLAN.md](AGENTFS_TEST_PLAN.md).

Minimum behavior tests:

- owner creates FS and receives owner authority
- writer can discover, accept, and attach after server-side share
- reader cannot commit
- writer can commit a clean local change
- stale commit is rejected
- accepted commit emits `commit.accepted`
- materialized commit emits `commit.materialized`
- second mount observes accepted commit after sync/watch
- metadata files cannot be committed as normal paths
- materialization failure produces non-ready state
- further commits are blocked while head materialization is failed

## First Implementation Slice

The first implementation should prove this flow:

```text
agent-a logs in
agent-a creates fs project with a SourceProfile
agent-a grants writer to agent-b
agent-a shares fs project with agent-b
agent-b logs in
agent-b sees fs project in fs available
agent-b accepts the share
agent-b attaches project
agent-b edits a normal file
agent-b commit apply --message "update file"
agent-a watch sees commit.accepted
agent-a sync/attach sees the materialized file
```

That is the minimum product proof for AgentFS governance.
