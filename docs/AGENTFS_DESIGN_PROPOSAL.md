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

## Truth Authority

The authority model is:

```text
accepted commit log = governance truth
backing source = materialized filesystem state
local mount = working copy
```

An accepted commit records what became true, who committed it, what paths changed, and which grant or policy allowed it.

The backing source is the materialized state used for sync and normal file access. If a commit is accepted but backing-source sync fails, the commit remains the governance record and the FS enters `syncing` or `error` until materialization catches up.

This avoids ambiguity between "the database says the commit happened" and "the remote files finished syncing".

## MVP Workflow

```text
agent creates fs
agent owns fs
agent grants another agent access
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
- grant-based attach and commit authority
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
- AgentDB, AgentGit, AgentOps

## Core Objects

| Object | Meaning |
| --- | --- |
| `Agent` | Actor that owns, attaches, or commits to an FS |
| `FS` | Agent-owned shared filesystem backed by a Section source |
| `Mount` | Local directory attached to an FS |
| `Grant` | Permission from one agent to another for an FS |
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

## Shared Metadata

AgentFS governance metadata must be shared across agents.

Local-only SQLite is not enough for:

- grants
- accepted commits
- FS events
- ownership
- current FS head

The MVP should define a shared metadata namespace under the backing source:

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

The local provider store may cache this metadata for performance, but the backing source metadata is the shared control-plane truth.

In a later implementation, this shared metadata may move to a dedicated service. The product contract should not depend on local-only state.

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
load shared grants
check commit authority
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
section agent identify
section agent register <name>

section fs create <name> --provider <provider> --opt key=value...
section fs list
section fs grant <fs> <agent> --role reader|writer|manager
section fs revoke <fs> <agent>
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

- Existing `source` commands remain lower-level infrastructure.
- `fs create` wraps source creation and AgentFS metadata creation.
- `fs attach` wraps local-root binding and initial sync.
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

Shared governance metadata lives in the backing source under `.section/agentfs/`. The local mount may hide or ignore that namespace in normal file workflows, but the control plane must be able to read and update it.

Do not materialize a large local `.section/*` control tree in MVP.

## Persistence Model

MVP shared metadata:

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
agent registers or identifies
agent runs fs create
Section creates backing source
Section writes shared .section/agentfs/fs.json
Section appends owner grant
Section appends fs.created event
```

### Attach FS

```text
agent requests attach
Section loads shared grants
Section checks read grant
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
Section loads shared head and grants
Section checks commit grant
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
- Add agent identity record.
- Add FS metadata record.
- Store shared FS metadata under `.section/agentfs/`.

### Phase 2: Grants And Attach

- Add shared grant log.
- Add `fs grant` / `fs revoke`.
- Enforce read grant on `fs attach`.
- Keep `source` commands as lower-level escape hatches.

### Phase 3: Commit Boundary

- Add `commit status`.
- Add `commit apply`.
- Append accepted commit records.
- Update FS head.
- Materialize accepted commits through existing sync/source machinery.
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

## Open Questions

- Should `fs create` hide provider options behind templates, or expose source options directly?
- How should Section prevent agents from accidentally editing `.section/agentfs/` metadata through the local mount?
- Should `commit apply` accept partial path scopes, or always commit all dirty local changes under the root?
- Should owner be an agent identity only, or can a human/user identity own an FS?

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
