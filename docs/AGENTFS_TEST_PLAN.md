# Section AgentFS Test Plan

## Purpose

This document defines the verification plan for the AgentFS MVP contract in
[AGENTFS_MVP_CONTRACT.md](AGENTFS_MVP_CONTRACT.md).

The goal is to make development testable before implementation starts. Every
test below should prove one contract boundary for:

```text
Agent / FS / Grant / Commit / Event
```

Messages, hook execution, proposal or approval workflows, path-scoped grants,
and `AGENTS.md` enforcement are out of scope for the current project. Tests may
assert that these surfaces are not implemented or not claimed.

## Verification Layers

| Layer | Scope | Evidence |
| --- | --- | --- |
| Unit contract | IDs, schemas, roles, paths, errors | crate unit tests |
| Provider store | local agent identity persistence | `section-provider` tests |
| Control plane | FS metadata, grants, attach, commit, events | `sectiond` tests |
| CLI integration | user-visible commands and JSON output | `section-cli` integration tests |
| End-to-end | two agents sharing one backing FS | CLI test with separate data dirs |
| Regression | existing source/path sync behavior | existing sync/path tests |

## Test Data Model

Use local filesystem OpenDAL provider for MVP tests:

```text
temp/
  remote/
  owner-data/
  writer-data/
  reader-data/
  owner.toml
  writer.toml
  reader.toml
  owner-root/
  writer-root/
  reader-root/
```

Each agent test config uses a distinct `data_dir` so agent identity and local
state are isolated. All agents point at the same backing `remote/` directory
through the same AgentFS source metadata.

## Unit Contract Tests

### ID Generation

| Case | Expected |
| --- | --- |
| generate agent id | matches `agt_[0-9a-f]{32}` |
| generate fs id | matches `fs_[0-9a-f]{32}` |
| generate commit id | matches `cmt_[0-9a-f]{32}` |
| generate grant id | matches `grt_[0-9a-f]{32}` |
| generate event id | matches `evt_[0-9]{13}_[0-9a-f]{16}` |
| generate many IDs | no duplicate in test sample |

### Path Validation

| Case | Expected |
| --- | --- |
| empty path | accepted as FS root |
| `docs/readme.md` | accepted |
| `/docs/readme.md` | rejected |
| `docs//readme.md` | rejected |
| `docs/../secret` | rejected |
| `.section/agentfs/fs.json` | rejected as reserved metadata |
| `.section/root.json` | allowed only as local discovery metadata, not shared AgentFS metadata |

### Role And Capability Mapping

| Role | Expected capabilities |
| --- | --- |
| `owner` | `read`, `commit`, `manage` |
| `reader` | `read` |
| `writer` | `read`, `commit` |
| `manager` | `read`, `manage` |

Additional checks:

- unknown role fails with a typed error,
- `manager` cannot grant `owner`,
- owner grant cannot be revoked.

### Metadata Schema Round Trips

Each JSON record must serialize and deserialize with `schema_version: 1`:

- `fs.json`
- grant record
- commit record
- `heads/current.json`
- event record
- head lock record

Invalid or missing required fields must fail with `malformed_shared_metadata`.

### Error Shape

Every AgentFS JSON CLI failure returns:

```json
{
  "error": {
    "code": "grant_denied",
    "message": "...",
    "retryable": false
  }
}
```

Required codes:

- `unknown_agent`
- `unknown_fs`
- `grant_denied`
- `stale_base`
- `reserved_metadata_path`
- `materialization_failed`
- `malformed_shared_metadata`
- `metadata_write_conflict`

## Provider Store Tests

| Case | Steps | Expected |
| --- | --- | --- |
| register agent | register `agent-a` | identity persisted with `agt_` id |
| identify agent | reopen store and identify | same id and name returned |
| replace display name | register same local identity with new name | id remains stable, name updates |
| isolated agents | use two data dirs | distinct agent ids |
| missing identity | identify before register | returns absent identity or `unknown_agent` at command boundary |

## Control Plane Tests

### FS Creation

| Case | Steps | Expected |
| --- | --- | --- |
| owner creates FS | agent registered, `fs create project` | source exists, `fs.json` exists, head exists with null commit |
| owner grant | create FS | owner grant exists with `owner` role |
| create event | create FS | `fs.created` event exists |
| duplicate source name | create existing FS/source | stable error; no partial metadata overwrite |
| create without agent | no local identity | `unknown_agent` |

### FS Listing And Lookup

| Case | Steps | Expected |
| --- | --- | --- |
| list created FS | create one FS | list returns FS metadata |
| ignore non-AgentFS source | add plain source | `fs list` does not treat it as FS |
| malformed metadata | corrupt `fs.json` | `malformed_shared_metadata` |
| lookup by id | use `fs_id` | resolves FS |
| lookup by name | use FS name/source name | resolves FS |

### Grant Management

| Case | Steps | Expected |
| --- | --- | --- |
| owner grants writer | `fs grant project agent-b --role writer` | writer grant stored and `grant.created` event emitted |
| manager grants reader | owner grants manager, manager grants reader | reader grant stored |
| reader cannot grant | reader runs grant command | `grant_denied` |
| manager cannot grant owner | manager grants owner | `grant_denied` |
| revoke grant | owner revokes writer | grant has `revoked_at_ms`, `grant.revoked` emitted |
| revoked writer attach | revoke then attach | `grant_denied` |
| revoked writer commit | revoke then commit | `grant_denied` |

### Attach

| Case | Steps | Expected |
| --- | --- | --- |
| owner attach | owner attaches own FS | local binding created, root marker written |
| writer attach | writer has writer grant | attach succeeds and syncs current materialized files |
| reader attach | reader has reader grant | attach succeeds |
| unknown agent attach | no identity | `unknown_agent` |
| no grant attach | registered but ungranted agent | `grant_denied` |
| marker contents | inspect `.section/root.json` | includes `schema_version`, `fs_id`, `source_id`, `agent_id`, `base_commit_id` |
| attach event | attach succeeds | `fs.attached` event emitted |
| failed materialized head | head commit is `failed_to_materialize` | attach reports non-ready state |

### Commit Apply

| Case | Steps | Expected |
| --- | --- | --- |
| writer commits file create | edit `docs/a.md`, apply message | commit accepted, head updated, remote file materialized |
| writer commits update | edit existing file | path op is `update` |
| writer commits delete | delete existing file | path op is `delete` |
| reader cannot commit | reader edits then apply | `grant_denied`, no commit record |
| empty message | `--message "   "` | rejected before metadata write |
| empty commit | no dirty paths | rejected before metadata write |
| stale base | local marker base differs from head | `stale_base`, no new commit |
| reserved metadata | dirty `.section/agentfs/fs.json` | ignored in dirty detection and rejected if explicitly included |
| malformed head | corrupt `heads/current.json` | `malformed_shared_metadata` |
| pending materialization | current head pending or failed | commit blocked with `materialization_failed` |

### Materialization

| Case | Steps | Expected |
| --- | --- | --- |
| successful materialization | commit accepted | commit state becomes `materialized`, event `commit.materialized` |
| materialization failure | make backing source unwritable or invalid | commit remains head, state becomes `failed_to_materialize`, event emitted |
| retry materialization | retry failed commit | same commit id used; no duplicate accepted commit |
| block next commit | head failed materialization | next `commit apply` fails |

### Events

| Case | Steps | Expected |
| --- | --- | --- |
| event IDs sort | create multiple events | lexical sort matches creation order by ms prefix |
| replay after id | list events after first id | later events returned |
| immutable event | write event then replay | existing event content unchanged |
| commit event data | commit accepted | event subject is commit id, actor is committing agent |
| grant event data | grant created/revoked | event subject is grant id |

### Reserved Metadata Sync

| Case | Steps | Expected |
| --- | --- | --- |
| remote scan sees `.section/agentfs` | run source sync | AgentFS metadata is not treated as user content |
| local scan sees `.section` | run source sync | `.section` remains skipped locally |
| remote manifest cache | metadata exists remotely | manifest excludes `.section/agentfs/**` |
| inventory accelerator | inventory includes `.section/agentfs/fs.json` | reserved path is filtered |

## CLI Integration Tests

### Agent Commands

```text
section agent register agent-a
section agent identify
```

Expected:

- normal output is human-readable,
- `--json` output includes `ok: true` and `agent`,
- repeated register returns the same agent id,
- identify before register fails or returns a clear absent identity according to final command contract.

### FS Commands

```text
section fs create project --provider fs --opt root=/tmp/remote
section fs list
section fs grant project agt_... --role writer
section fs revoke project agt_...
section fs attach project /tmp/writer-root
section fs status project
```

Expected:

- JSON output is stable and machine-readable,
- grant-denied failures use the AgentFS error shape,
- status reports FS head, role, base commit, dirty state, and materialization state.

### Commit Commands

```text
section commit status /tmp/writer-root
section commit apply /tmp/writer-root --message "update file"
```

Expected:

- status reports dirty paths and stale-base state,
- apply accepts all dirty paths under the attached root,
- accepted commit id is returned,
- failure cases return typed AgentFS errors.

### Watch

```text
section watch /tmp/owner-root --once
```

Expected:

- source/path events continue to work,
- AgentFS events are identifiable by kind or stream,
- `commit.accepted` is observable after commit.

## End-To-End MVP Proof

This is the minimum product proof:

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | agent-a | register | agent identity persisted |
| 2 | agent-a | create FS `project` | owner grant and `fs.created` event |
| 3 | agent-a | attach owner root | root marker with null base head |
| 4 | agent-b | register | distinct identity |
| 5 | agent-a | grant writer to agent-b | writer grant and event |
| 6 | agent-b | attach writer root | materialized files synced |
| 7 | agent-b | edit normal file | local dirty draft only |
| 8 | agent-b | commit apply | commit accepted, head advances |
| 9 | agent-a | watch or event replay | sees `commit.accepted` |
| 10 | agent-a | sync/attach | materialized file appears |

## Regression Tests

Run existing tests to prove AgentFS does not break source/path infrastructure:

```bash
cargo test -p section-provider
cargo test -p sectiond
cargo test -p section-cli --test path_control_plane
cargo test -p section-cli --test sync_control_plane
cargo test -p section-cli --tests
```

## Out-Of-Scope Assertions

The MVP must not imply unsupported behavior:

| Surface | Assertion |
| --- | --- |
| Messages | no `message` or conversation command is added |
| Hooks | event names may be reserved, hook execution is not implemented |
| `AGENTS.md` | file is synced as normal content, Markdown rules are not enforced |
| path-scoped grants | all grants are FS-wide |
| proposals/approvals | direct commit is the only MVP mutation path |
| AgentDB/Git/Ops | not implemented in this repo |

## Completion Gate

The AgentFS MVP is complete only when:

- all unit, control-plane, CLI, and end-to-end cases above have tests,
- every required error code is asserted at least once,
- accepted commit metadata and materialized backing files are both verified,
- existing source/path regression tests pass,
- unimplemented out-of-scope surfaces are not exposed as working features.
