# Section AgentFS Development Readiness

## Purpose

This document lists the contracts that must be defined before implementation starts.

It is a development gate for [AGENTFS_DESIGN_PROPOSAL.md](AGENTFS_DESIGN_PROPOSAL.md). The concrete answers live in [AGENTFS_MVP_CONTRACT.md](AGENTFS_MVP_CONTRACT.md). The goal is to avoid starting code before the authority model, shared metadata model, and control-plane behavior are precise enough to test.

## Development Gate

Development can start when the Phase 1 contracts below are answered in docs.

Do not start by implementing hooks, approval workflows, or `AGENTS.md` enforcement. Those are later layers. Messages are out of scope for the current project. The first build proves the AgentFS core:

```text
Agent / FS / Grant / Commit / Event
```

## Phase 1 Required Definitions

### 1. Terms And Identity

Define stable identifiers and naming rules:

| Term | Required Definition |
| --- | --- |
| `agent_id` | Format, persistence location, uniqueness, display name rules |
| `fs_id` | Format, relationship to source name, rename behavior |
| `mount_id` | Whether needed in MVP or derived from local root |
| `commit_id` | Format, ordering, collision strategy |
| `event_id` | Format and monotonicity expectations |

Minimum decision:

```text
IDs should be stable, opaque strings. Human-readable names are metadata, not identity.
```

### 2. Authority Model

Define the truth hierarchy exactly:

```text
accepted commit log = governance truth
backing source = materialized filesystem state
local mount = working copy
```

Required answers:

- What makes a commit accepted?
- What happens if commit metadata writes succeed but file materialization fails?
- What state is shown while materialization is incomplete?
- Which state can watchers rely on as authoritative?
- Can a backing source be edited outside Section, and how is that detected?

Minimum decision:

```text
Accepted commit metadata is authoritative for governance.
Backing source sync can lag and is reported as syncing/error.
```

### 3. Shared Metadata Layout

Define exact backing-source paths and ownership.

MVP namespace:

```text
.section/agentfs/fs.json
.section/agentfs/heads/current.json
.section/agentfs/grants/<grant_id>.json
.section/agentfs/commits/<commit_id>.json
.section/agentfs/events/<event_id>.json
.section/agentfs/locks/head.json
```

Required answers:

- Are these files visible in attached mounts?
- Are they excluded from normal sync diffs?
- How does Section prevent ordinary commits from modifying metadata files?
- Are metadata records immutable or mutable?
- How are partial writes avoided or recovered?

Minimum decision:

```text
.section/agentfs/* is reserved control metadata.
Normal file commits cannot modify it.
```

### 4. Metadata Schemas

Define versioned JSON schemas before implementation.

Required records:

- `fs.json`
- grant record
- commit record
- event record
- `heads/current.json`

Each schema must define:

- required fields
- optional fields
- version field
- timestamp format
- actor field
- path format
- error/failure representation

Minimum fields:

```text
fs: schema_version, fs_id, name, owner_agent_id, created_at_ms
grant: schema_version, grant_id, fs_id, agent_id, role, granted_by, created_at_ms, revoked_at_ms
commit: schema_version, commit_id, fs_id, parent_commit_id, agent_id, summary, paths, created_at_ms, materialization_state
event: schema_version, event_id, fs_id, kind, actor_agent_id, subject_id, path, created_at_ms
head: schema_version, fs_id, commit_id, updated_at_ms
```

### 5. Grant Semantics

Define exactly what roles allow.

MVP roles:

| Role | Capabilities |
| --- | --- |
| `owner` | `read`, `commit`, `manage` |
| `reader` | `read` |
| `writer` | `read`, `commit` |
| `manager` | `read`, `manage` |

Required answers:

- Can `manager` grant `owner`?
- Can ownership transfer in MVP?
- Can `owner` be revoked?
- Are grants FS-wide only in MVP?
- What happens when a grant is revoked while an agent has a mount?

Minimum decision:

```text
MVP grants are FS-wide.
Owner cannot be revoked.
Ownership transfer is deferred.
Revoked agents cannot attach or commit after revocation is observed.
```

### 6. Commit Semantics

Define commit behavior as a testable contract.

Required answers:

- Does `commit apply` commit all dirty paths or selected paths?
- How is a dirty path discovered?
- What is the base for freshness checks?
- What happens on stale base?
- What files are forbidden from commit?
- Is a summary required?
- Can an empty commit exist?

Minimum decision:

```text
MVP commit applies all dirty paths under the attached root.
Commit requires a non-empty summary.
Commit is rejected if the local base is stale.
```

### 7. Materialization Semantics

Define how accepted commits become backing-source files.

Required answers:

- Does commit metadata append before or after file upload?
- What is the retry behavior?
- How is `failed_to_materialize` represented?
- Can another commit proceed while materialization is incomplete?
- How does attach handle a partially materialized head?

Minimum decision:

```text
Commit metadata is appended first, then materialization runs.
If materialization fails, FS state is error/syncing until repair.
Further commits should block until the head is materialized.
```

### 8. Event Semantics

Define the event stream before implementing watch integration.

MVP event kinds:

```text
fs.created
fs.attached
grant.created
grant.revoked
commit.accepted
commit.materialization_failed
commit.materialized
fs.error
```

Required answers:

- Are events append-only?
- Can events be replayed from an offset?
- Are existing source/path events and AgentFS events merged or separate?
- What ordering is guaranteed?

Minimum decision:

```text
AgentFS events are append-only and replayable by event_id.
Watch may merge source/path events and AgentFS events, but event kind must identify the stream.
```

### 9. Local Mount Contract

Define what attach means.

Required answers:

- Does attach create a source binding?
- Does attach always sync immediately?
- What does `.section/root.json` contain for AgentFS?
- Can multiple local roots attach to the same FS on one machine?
- Are raw local edits allowed even for reader mounts?

Minimum decision:

```text
Attach creates a local working copy and writes discovery metadata.
Section does not prevent raw local edits in a normal directory.
Only commit is governed.
```

### 10. Source Compatibility

Define how AgentFS maps to existing `source` behavior.

Required answers:

- Does each FS own exactly one source?
- Can an existing source be upgraded into an FS?
- Can `source sync` bypass AgentFS governance?
- Are low-level `source` commands still documented as escape hatches?

Minimum decision:

```text
MVP FS wraps one backing source.
Existing source commands remain lower-level escape hatches and are not the AgentFS governance surface.
```

### 11. Error Model

Define user-facing and JSON errors.

Required errors:

- unknown agent
- unknown FS
- grant denied
- stale base
- reserved metadata path
- materialization failed
- malformed shared metadata
- metadata write conflict

Each error must define:

- stable code
- human message
- JSON shape
- whether retry is safe

### 12. Test Contract

Define the first behavior tests before implementation.

Minimum tests:

- owner creates FS and receives owner authority
- writer can attach after grant
- reader cannot commit
- writer can commit clean local change
- stale commit is rejected
- accepted commit emits event
- second mount observes accepted commit
- metadata files cannot be committed as normal paths
- materialization failure produces non-ready state

## Explicitly Deferred Definitions

These must not block core development:

- hook execution model
- hook trust model
- `AGENTS.md` parsing or enforcement
- proposal/approval workflow
- path-scoped grants
- ownership transfer
- organization/team identity
- dedicated control-plane service
- semantic search
- sandbox runtime integration

## Recommended First Implementation Slice

The smallest useful implementation slice is:

```text
agent register
fs create
fs grant
fs attach
commit status
commit apply
watch AgentFS events
```

This slice should prove:

```text
two agents can share one FS;
one owner grants one writer;
the writer commits a local change;
the owner observes the accepted mutation.
```

## Exit Criteria

The design is ready for development when:

- all Phase 1 definitions are answered in docs
- [AGENTFS_MVP_CONTRACT.md](AGENTFS_MVP_CONTRACT.md) is current
- schemas exist for shared metadata
- CLI command contracts are stable enough for tests
- multi-agent happy path is described end to end
- stale-base and materialization-failure behavior are described
- deferred layers remain explicitly out of MVP
