# Section AgentFS MVP Contract

## Purpose

This document answers the Phase 1 readiness questions in [AGENTFS_DEVELOPMENT_READINESS.md](AGENTFS_DEVELOPMENT_READINESS.md).

It is the development contract for the first implementation slice:

```text
Agent / FS / Grant / Commit / Event
```

Messages, hooks, proposal/approval workflows, path-scoped grants, and `AGENTS.md` enforcement are out of scope for this contract.

## 1. Terms And Identity

IDs are stable, opaque strings. Display names are metadata and can change.

| Term | Format | Notes |
| --- | --- | --- |
| `agent_id` | `agt_` + 32 lowercase hex chars | Generated once per agent identity and persisted locally |
| `fs_id` | `fs_` + 32 lowercase hex chars | Generated at FS creation; independent from source name |
| `mount_id` | derived from `fs_id + canonical_local_root` in MVP | No persistent mount id required in the first implementation |
| `commit_id` | `cmt_` + 32 lowercase hex chars | Generated before commit metadata is written |
| `event_id` | `evt_` + 13 digit epoch-ms + `_` + 16 lowercase hex chars | Sortable by timestamp prefix; tie-breaker is the random suffix |
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
accepted commit log = governance truth
backing source = materialized filesystem state
local mount = working copy
```

A commit is accepted when:

1. the committing agent has an active `commit` capability,
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

## 3. Shared Metadata Layout

AgentFS shared metadata lives under the backing source:

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
    locks/
      head.json
```

Rules:

- `.section/agentfs/**` is reserved.
- Normal commits cannot modify reserved metadata paths.
- The local mount may expose `.section/root.json` for discovery.
- The local mount should not expose `.section/agentfs/**` as normal user content when it can avoid it.
- If reserved metadata appears in a working copy, `commit apply` must ignore it for dirty detection and reject explicit attempts to commit it.

Metadata files are rewritten as whole JSON documents. The MVP does not use JSONL append files because object backends do not provide portable append semantics.

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

- The lock protects grant changes, head updates, and commit acceptance.
- A stale lock can be replaced after `expires_at_ms`.
- If the lock cannot be acquired, commands return `metadata_write_conflict`.
- The first implementation can keep lock semantics conservative and fail closed.

## 4. Metadata Schemas

All records include `schema_version: 1`.

### `fs.json`

```json
{
  "schema_version": 1,
  "fs_id": "fs_...",
  "name": "project",
  "owner_agent_id": "agt_...",
  "source_name": "project",
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
  "agent_id": "agt_...",
  "summary": "Update docs",
  "paths": [
    {
      "path": "docs/readme.md",
      "kind": "file",
      "op": "update",
      "local_version": "sha256:...",
      "previous_version": "sha256:..."
    }
  ],
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

- `.section/root.json` records `base_commit_id` for the attached working copy.
- `heads/current.json` records current shared head.
- commit is rejected with `stale_base` when these differ.

Rules:

- summary is required and must be non-empty after trimming whitespace.
- empty commits are rejected.
- commits containing `.section/agentfs/**` are rejected.
- reserved metadata paths are ignored in dirty detection.
- external backing-source drift is treated as conflict or stale state before commit acceptance.

## 7. Materialization Semantics

Commit acceptance writes governance metadata first:

1. acquire head lock,
2. verify grant and freshness,
3. write commit record with `materialization_state: "pending"`,
4. update `heads/current.json`,
5. write `commit.accepted` event,
6. release head lock,
7. materialize file changes to backing source,
8. update commit materialization state,
9. write `commit.materialized` or `commit.materialization_failed` event.

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
- clients sort by `event_id`,
- clients can resume from the last seen `event_id`.

Ordering:

- events emitted under the head lock are ordered relative to commit/grant head mutations.
- materialization events happen after the accepted commit event.
- source/path events and AgentFS events may be merged in `watch`, but each output event must include a stream or kind that identifies its origin.

## 9. Local Mount Contract

`fs attach` creates a local working copy.

Attach behavior:

- checks `read` capability,
- creates or updates the source local-root binding,
- writes `.section/root.json`,
- syncs current materialized backing-source state into the local root,
- records `base_commit_id` in `.section/root.json`.

`root.json` fields:

```json
{
  "schema_version": 1,
  "fs_id": "fs_...",
  "source_id": "project",
  "agent_id": "agt_...",
  "local_root": "/abs/path/project",
  "base_commit_id": "cmt_...",
  "control_plane_endpoint": "local-cli"
}
```

Rules:

- multiple local roots may attach to the same FS if their paths are distinct.
- reader mounts can be edited locally by the OS, but reader commits are denied.
- attach should fail or report non-ready state when current head is not materialized.

## 10. Source Compatibility

MVP FS wraps exactly one backing source.

Rules:

- `fs create` creates or reuses one source as backing storage.
- upgrading an existing source into an FS is deferred.
- low-level `source` commands remain lower-level escape hatches.
- `source sync` can mutate materialized state outside AgentFS governance and may create drift.
- AgentFS commands are the governance surface; source commands are infrastructure.

Documentation must not imply that low-level source commands preserve AgentFS governance.

## 11. Error Model

Errors must have stable codes for JSON output.

| Code | Meaning | Retry |
| --- | --- | --- |
| `unknown_agent` | Agent identity is missing or not registered | no, register first |
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
    "retryable": false
  }
}
```

## 12. Test Contract

The complete implementation test plan lives in
[AGENTFS_TEST_PLAN.md](AGENTFS_TEST_PLAN.md).

Minimum behavior tests:

- owner creates FS and receives owner authority
- writer can attach after grant
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
agent-a registers
agent-a creates fs project
agent-a grants writer to agent-b
agent-b attaches project
agent-b edits a normal file
agent-b commit apply --message "update file"
agent-a watch sees commit.accepted
agent-a sync/attach sees the materialized file
```

That is the minimum product proof for AgentFS governance.
