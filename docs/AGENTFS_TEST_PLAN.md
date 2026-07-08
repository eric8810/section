# Section AgentFS Test Plan

## Purpose

This document defines the verification plan for the AgentFS MVP contract in
[AGENTFS_MVP_CONTRACT.md](AGENTFS_MVP_CONTRACT.md), and the end-to-end gates
for the next AgentFS features defined in
[AGENTFS_DESIGN_PROPOSAL.md](AGENTFS_DESIGN_PROPOSAL.md).

The goal is to make development testable before implementation starts. Every
test below should prove one contract boundary for:

```text
Agent / FS / Grant / Commit / Event
```

Hook execution, proposal or approval workflows, path-scoped grants, and
`AGENTS.md` enforcement are out of scope for the current MVP slice. Future E2E
gates below define the product proof required when those features are built.

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
  control-service-data/
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
through the same server-managed `SourceProfile`.

Sharing tests must run through Section Control Service. Endpoint tests start a
`sectiond serve` HTTP Control Service. Client configs only know the endpoint;
the service owns agent identity, installation identity, grants, shares, source
profiles, and short-lived sync credentials.

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
| `.section/root.json` | local discovery metadata; rejected as shared user content |
| `.section/user-note.txt` | rejected as reserved metadata |

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
| owner creates FS | agent logged in, source profile exists, `fs create project` | FS exists, source profile is bound, owner grant exists, head exists with null commit |
| owner grant | create FS | owner grant exists with `owner` role |
| create event | create FS | `fs.created` event exists |
| duplicate source name | create existing FS/source | stable error; no partial metadata overwrite |
| create without agent | no local identity | `unknown_agent` |
| metadata init failure | backing source becomes unwritable during create | create fails; Control Service rows, local source cache, and remote `.section` metadata are cleaned; retry create succeeds |

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
| marker contents | inspect `.section/root.json` | includes `schema_version`, `fs_id`, `source_profile_id`, `agent_id`, `installation_id`, `base_commit_id` |
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
| successful materialization | commit accepted | commit state becomes `materialized`, event `commit.materialized` includes materialization state and changed paths |
| materialization failure | make backing source unwritable or invalid | commit remains head, state becomes `failed_to_materialize`, events `commit.materialization_failed` and `fs.error` are emitted |
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
| remote scan sees `.section` | run source sync | Section metadata is not treated as user content |
| local scan sees `.section` | run source sync | `.section` remains skipped locally |
| remote manifest cache | metadata exists remotely | manifest excludes `.section/**` |
| inventory accelerator | inventory includes `.section/agentfs/fs.json` | reserved path is filtered |

## CLI Integration Tests

### Agent Commands

```text
section agent login
section agent identify
```

Expected:

- normal output is human-readable,
- `--json` output includes `ok: true` and `agent`,
- repeated login identifies the same agent when the same account is used,
- identify before login fails or returns a clear absent identity according to final command contract.

### FS Commands

```text
section fs create project --source-profile test-profile
section fs list
section fs grant project agt_... --role writer
section fs revoke project agt_...
section fs share project agt_...
section fs available
section fs accept shr_...
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

## End-To-End Product Definition

End-to-end tests are product proofs, not control-plane unit tests. They must
exercise the system from an agent's point of view and prove this product model:

```text
agent creates FS
agent owns FS
agent grants other agents access
agents attach FS as normal files
agents edit locally
agents commit through policy
accepted commits become shared truth
all agents can observe accepted FS mutations
```

An AgentFS E2E passes only when it proves the difference between local work and
shared truth:

```text
local edit != shared truth
accepted commit == shared truth mutation
```

### E2E Test Boundary

E2E tests must use the released CLI shape and filesystem behavior:

- run the `section` binary through the CLI test harness,
- run a Section Control Service endpoint or local harness for sharing and credential tests,
- use separate config files and `data_dir` values for each agent,
- use one shared backing source for the FS,
- use separate local roots for each attached agent,
- perform user work through ordinary file operations under local roots,
- avoid calling `sectiond` Rust APIs directly.

E2E tests may inspect shared metadata files under `.section/agentfs/` as an
oracle, but the primary assertions should be made through CLI output and
observable filesystem state.

Product E2E for events must use `section watch` or `section fs events`.
Metadata file inspection is only a secondary oracle.

### Product Questions Covered By E2E

Each complete E2E suite must answer these questions from the agent perspective:

| Product Question | E2E Evidence |
| --- | --- |
| Which FS do I own? | owner can create, list, status, attach, grant |
| Which FS can I access? | granted writer/reader can attach; ungranted agent cannot |
| What files can I read? | attached roots materialize accepted shared truth |
| What files can I change locally? | agents can edit local files with ordinary filesystem operations |
| What changes am I allowed to commit? | commit succeeds or fails according to active grant capability |
| What changed while I was away? | second agent can observe a newer accepted commit and materialized file |
| Which mutation affected truth? | commit record and head identify the accepted mutation |
| Which agent changed it? | commit record and event actor identify the committing agent |
| Which policy allowed it? | MVP checks active grant role/capabilities; final proof records the authorizing grant or owner authority |
| What happens to rejected work? | local draft remains local and backing source/head remain unchanged |

### Required E2E Fixtures

Every AgentFS E2E should start from an isolated fixture:

```text
temp/
  remote/
  owner-data/
  writer-data/
  reader-data/
  stranger-data/
  owner.toml
  writer.toml
  reader.toml
  stranger.toml
  owner-root/
  writer-root/
  reader-root/
  stranger-root/
```

All agents use the same backing `remote/` through the local filesystem provider.
Each agent has a distinct `data_dir` so identity, grants observed by commands,
and local root bindings cannot accidentally share local state.

### Required E2E Scenarios

#### 1. Owner Creates A Governed FS

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | `agent login` | authenticated `agt_` identity and installation |
| 2 | owner | `fs create project --source-profile test-profile` | `fs_` created; owner grant exists |
| 3 | owner | `fs list` and `fs status project` | owner can discover and inspect FS |
| 4 | owner | inspect `remote/.section/agentfs/` | `fs.json`, head, owner grant, `fs.created` event exist |

This proves agent-owned filesystem creation and initial governance metadata.

#### 2. Writer Commit Becomes Shared Truth

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | `fs attach project owner-root` | owner root marker records null base head |
| 2 | writer | `agent login` | distinct `agt_` identity and installation |
| 3 | owner | `fs grant project <writer_id> --role writer` | writer has `read` and `commit` capability |
| 4 | owner | `fs share project <writer_id>` | server-side share exists |
| 5 | writer | `fs available` then `fs accept <share_id>` | writer accepts the FS through the control service |
| 6 | writer | `fs attach project writer-root` | writer gets short-lived sync credential and materializes current shared truth |
| 7 | writer | write `writer-root/docs/note.txt` | remote file does not exist yet |
| 8 | writer | `commit status writer-root` | dirty path reports `docs/note.txt` as create |
| 9 | writer | `commit apply writer-root --message "add note"` | commit id returned; materialization succeeds |
| 10 | test | inspect head and commit metadata | head points to commit; commit actor is writer |
| 11 | test | inspect remote file | `remote/docs/note.txt` exists with writer content |
| 12 | owner | attach a fresh root or sync existing root | owner observes writer's accepted content |

This proves the core product statement: local draft is not shared truth until a
policy-accepted commit advances the FS head.

#### 3. Reader Can Read But Cannot Commit

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | grant reader to `reader` | reader has `read` only |
| 2 | reader | attach `reader-root` | attach succeeds |
| 3 | reader | edit `reader-root/draft.txt` | edit stays local |
| 4 | reader | `commit apply reader-root --message "reader draft"` | `grant_denied` |
| 5 | test | inspect head, commits, remote file | no accepted commit; remote unchanged |

This proves grants govern acceptance into shared truth, not local write syscalls.

#### 4. Ungranted Agent Cannot Attach

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | stranger | log in without grant or share | local identity exists |
| 2 | stranger | `fs attach project stranger-root` | `grant_denied` |
| 3 | test | inspect `stranger-root` | no AgentFS root marker is written |

This proves access to an FS is not implicit from knowing the backing source.

#### 5. Grant Revocation Or Downgrade Takes Effect

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | grant writer to `writer` | writer can attach and commit |
| 2 | owner | grant reader to same `writer`, or revoke writer | active commit capability removed |
| 3 | writer | edit an already attached root | local draft can exist |
| 4 | writer | `commit apply` | `grant_denied` |
| 5 | test | inspect remote and head | draft did not become shared truth |

This proves current active grants, not historical local access, authorize
commits.

#### 6. Stale Writer Cannot Overwrite New Truth

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | grant writer to `writer-a` and `writer-b` | both can attach |
| 2 | writer-a | attach at head `H0` | marker base is `H0` |
| 3 | writer-b | attach at head `H0` | marker base is `H0` |
| 4 | writer-a | edit and commit | head advances to `H1` |
| 5 | writer-b | edit and commit from stale root | `stale_base` |
| 6 | test | inspect remote/head | writer-a content remains current truth |

This proves accepted commits are based on the current FS head and stale local
work cannot silently replace newer truth.

#### 7. Shared Metadata Is Not User Content

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | writer | attach FS | local root created |
| 2 | writer | inspect user-visible local root | `.section/**` is not materialized as ordinary content |
| 3 | writer | create normal files and commit | commit paths exclude `.section/**` |
| 4 | owner | attach fresh root | owner sees only user files plus local root marker |

This proves governance metadata is shared control state, not ordinary FS user
content.

#### 8. Observable Mutation Trail

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | create FS | `fs.created` observable |
| 2 | owner | grant writer | `grant.created` observable |
| 3 | writer | commit | `commit.accepted` and `commit.materialized` observable |
| 4 | observer | replay events or watch | events identify kind, actor, subject, and FS |

This proves accepted mutations are observable. The current implementation may
verify event files directly; the final product gate requires CLI event replay or
watch.

#### 9. Governance Boundary Hardening

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | `fs create` over a non-empty backing source | rejected; no source registration |
| 2 | owner | attach backing source root as working root | rejected; no local marker |
| 3 | owner | metadata initialization fails during `fs create` | rejected; service rows and local source cache are rolled back; retry works |
| 4 | writer | commit symlink path | rejected; target outside root does not materialize |
| 5 | writer | inspect `fs attach --json` | source options and credentials are not exposed |

This proves common filesystem edge cases do not collapse the product boundary.

#### 10. Manager Can Manage But Cannot Commit

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | grant manager to `manager` | manager has `read` and `manage` capability |
| 2 | manager | grant reader or writer to another agent | grant succeeds and event is emitted |
| 3 | manager | attach and edit local root | local draft can exist |
| 4 | manager | `commit apply` | `grant_denied` |
| 5 | test | inspect head and remote | manager draft did not become shared truth |

This proves manage authority is distinct from commit authority.

#### 11. Backing Source Drift Does Not Become Silent Truth

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | create, attach, and commit baseline file | head points to baseline commit |
| 2 | external process | mutate backing file directly outside AgentFS | backing source now differs from governed base |
| 3 | writer | `commit status` or `commit apply` from old local root | `remote_drift` is surfaced |
| 4 | test | inspect head and commit records | no accepted commit silently blesses external drift |

This proves external backing-source changes are not governed commits. The E2E
must prove the mutation is not silently accepted as AgentFS truth.

#### 12. Commit Snapshot Matches Materialized Bytes

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | writer | edit local file to version A | dirty path reports version A |
| 2 | writer | start commit flow | commit preflight records intended path/version |
| 3 | writer or test harness | mutate the local file to version B during commit window | commit either uses a frozen snapshot or rejects |
| 4 | test | compare commit record and remote bytes | accepted commit metadata matches materialized bytes |

This proves the accepted commit record describes what actually became shared
truth. If the implementation cannot create this race deterministically, the E2E
can use a test hook or slow provider later; until then this remains a required
product-complete scenario.

#### 13. Event Write Failure Does Not Advance Commit Head

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | create and attach FS | head has no commit |
| 2 | test harness | make AgentFS event writes fail | event stream cannot accept new event |
| 3 | owner | `commit apply` dirty local file | command fails |
| 4 | test | inspect head and backing source | head is unchanged; user file is not materialized |

This proves a `commit.accepted` event is required before head advances. It does
not yet prove every Control Service mutation and every backing-source mirror
write are one distributed transaction.

#### 14. Materialization Failure Blocks New Commits

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | writer | commit a dirty path while backing source fails materialization | commit remains accepted with `failed_to_materialize`; `commit.materialization_failed` and `fs.error` are emitted |
| 2 | writer | attempt another commit | command fails before accepting a new commit |
| 3 | owner or writer | inspect status | FS reports non-ready materialization state |
| 4 | test | inspect head | head remains on the failed accepted commit |

This proves governance truth can advance independently from file
materialization, and that the system does not accept follow-up mutations while
the head is not materialized.

#### 15. Granted Agent Bootstrap Is Product-Visible

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | grant writer to `writer` | writer is authorized on the FS |
| 2 | owner | create a server-side share for `writer` | share record exists in Section Control Service |
| 3 | writer | log in and run `fs available` | writer sees the shared FS |
| 4 | writer | `fs accept <share_id>` | service validates share and grant, then returns source profile and short-lived credential |
| 5 | writer | attach | attach succeeds using server-issued credential |
| 6 | test | inspect writer local store | accepted FS and credential binding exist; source long-lived key is absent |

This proves grants lead to usable access from the grantee's perspective. The
required product-complete E2E is server-side share discovery and accept.

#### 16. Local Root Identity Is Stable

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | writer | attach using one spelling of a path | root marker and local binding are written |
| 2 | writer | run commit/status using an equivalent canonical path | command resolves the same attached root |
| 3 | writer | try overlapping parent/child roots | attach or bind rejects ambiguous overlap |
| 4 | test | inspect local bindings | one FS cannot accidentally treat another root marker as user content |

This proves local root identity does not depend on fragile path spelling or
overlapping directory layouts.

### 后续功能 E2E 套件

后续 E2E 只从 Agent 视角证明产品能力。

规则：

- 每个后续功能至少有一条端到端测试。
- 主要断言通过 CLI、local root、remote materialized files、watch/events 完成。
- metadata 文件可以作为辅助 oracle，但不能替代用户可见行为。
- 需要制造 race、失败、hook 输出时，可以使用测试 harness，但最终断言仍然走公开命令。

执行顺序：

| 阶段 | 目标 | 场景 |
| --- | --- | --- |
| P0 | 治理真相不能错 | 16-24 |
| P1 | 多 Agent 使用体验完整 | 25-28 |
| P2 | 自动化和规则层 | 29-33 |

额外 fixture：

```text
temp/
  control-service-data/
  observer-data/
  manager-data/
  writer-b-data/
  credential-broker/
  hooks/
  hook-output/
  corrupt-remote/
```

测试 harness 可以提供：

- 可暂停 materialization 的 provider，用来测试 snapshot isolation。
- 可失败一次的 provider，用来测试 materialization repair。
- Section Control Service endpoint / harness，用来测试 share、discovery、grant、credential。
- 临时 hook script，用来记录输入 JSON、返回成功或失败。

#### 17. Trusted Dirty Base And Status

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | create FS and grant writer to `writer-a` and `writer-b` | both writers can attach |
| 2 | writer-a | attach at head `H0` using path spelling A | local mount state records canonical root and base `H0` |
| 3 | writer-a | run `fs status` using equivalent path spelling B | status resolves the same mount |
| 4 | writer-b | commit a file | head advances to `H1` |
| 5 | writer-a | edit local file and run `fs status --json` | `dirty: true`, `stale: true`, base `H0`, head `H1` |
| 6 | writer-a | `commit apply` | `stale_base`, no new commit |

This proves Section compares local work against the mounted base, not only the
current remote files.

#### 18. Commit Snapshot Isolation

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | writer | edit `docs/note.txt` to version A | `commit status` reports version A dirty |
| 2 | writer | start `commit apply` with a paused materialization provider | staging snapshot is created before metadata write |
| 3 | test harness | change live local file to version B before materialization resumes | live root now differs from staging |
| 4 | harness | resume materialization | remote file contains version A |
| 5 | test | inspect commit record and `fs status` | commit path hash matches version A; version B remains local dirty work |

This proves accepted commit metadata describes the bytes that became shared
truth.

#### 19. Materialization Repair

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | writer | commit with provider configured to fail materialization once | commit is accepted; head points to it; state is `failed_to_materialize` |
| 2 | writer | run another `commit apply` | rejected with `materialization_failed` |
| 3 | owner | `fs status --json` | non-ready materialization state is visible |
| 4 | writer | `commit repair <root> --commit <commit_id>` after provider recovers | same commit id becomes `materialized` |
| 5 | writer | run a new commit | new commit can proceed after repair |

This proves repair fixes the accepted head instead of creating a second truth.

#### 20. Metadata Validation And Bad Data Isolation

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | create valid FS `good` | status succeeds |
| 2 | test harness | create another source with malformed `.section/agentfs/fs.json` | corrupt source exists |
| 3 | owner | `fs status good --json` | succeeds; may include warning about unrelated bad metadata |
| 4 | owner | `fs status corrupt --json` | fails with `malformed_shared_metadata` |
| 5 | test harness | create two FS records with same source name | lookup by source name fails with `ambiguous_fs_ref` |
| 6 | owner | lookup by exact `fs_id` | exact id still resolves |

This proves bad metadata is isolated to the affected FS or lookup candidate.

#### 21. Event Immutability And Sequence Replay

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | create FS, grant writer, writer commits twice | multiple AgentFS events exist |
| 2 | observer | `fs events <fs> --json` | events have strictly increasing `seq` |
| 3 | observer | `fs events <fs> --after <seq>` | replay returns only later events |
| 4 | test harness | try to pre-create or overwrite an existing event id before a command writes | command fails or creates a different event; old event is unchanged |
| 5 | observer | replay again | event order is still by `seq`, not filesystem list order |

This proves events are append-only and replayable.

#### 22. Source And Path Guardrails

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | create AgentFS-backed source | source record is marked as AgentFS-owned |
| 2 | owner | run `source sync` on that source | rejected with AgentFS guardrail error |
| 3 | owner | run `source remove` or `source bind` | rejected with AgentFS guardrail error |
| 4 | owner | try any low-level force path | rejected; MVP has no low-level bypass |
| 5 | owner | run `path inspect/compare/resolve` under the root | rejected with AgentFS guardrail error |

This proves low-level source/path commands do not silently bypass AgentFS.

#### 23. JSON Error Contract

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | stranger | `fs attach project stranger-root --json` | `error.code = grant_denied`, `retryable = false` |
| 2 | writer | stale `commit apply --json` | `error.code = stale_base`, `retryable = true` |
| 3 | owner | status corrupt metadata with `--json` | `error.code = malformed_shared_metadata` |
| 4 | writer | commit while head failed materialization with `--json` | `error.code = materialization_failed`, `retryable = true` |
| 5 | test harness | hold metadata lock, then run grant or commit with `--json` | `error.code = metadata_write_conflict`, `retryable = true` |
| 6 | writer | commit a local path that is not UTF-8 | `error.code = non_utf8_path`, no accepted commit |

This proves Agent callers can make decisions from stable error codes.

#### 24. File/Dir Replacement Preflight

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | writer | commit file `docs/api` | file materializes |
| 2 | writer | replace `docs/api` with directory `docs/api/index.md` in one commit | rejected with `path_type_conflict` before metadata write |
| 3 | test | inspect head and remote after failure | head is unchanged; remote still has the original file |
| 4 | writer | delete a non-empty directory as an explicit delete commit | commit succeeds; remote subtree is removed cleanly |
| 5 | test | inspect working copy after delete | dirty paths are clean |

This proves commit acceptance rejects ambiguous same-path type replacement
before shared truth advances, and that explicit directory deletes materialize
cleanly.

#### 25. Local Root Identity And Overlap

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | writer | attach using `/tmp/project/../project` | mount stores canonical root |
| 2 | writer | run `fs status /tmp/project` | resolves the same `mount_id` |
| 3 | writer | attach child root under existing root | rejected |
| 4 | writer | attach parent root over existing root | rejected |
| 5 | writer | attach backing source root as working root | rejected |

This proves root identity is stable and nested roots cannot leak control files.

#### 26. Server Share And Accept

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | `fs grant project <writer_id> --role writer` | writer has commit authority |
| 2 | owner | `fs share project <writer_id>` | server-side share record is created |
| 3 | writer | `fs available --json` | writer sees the shared FS after login |
| 4 | writer | `fs accept <share_id>` | service validates identity, share, and grant |
| 5 | writer | `fs attach project writer-root` | source profile and short-lived sync credential are bound locally |
| 6 | writer | inspect local store through `fs status writer-root --json` | accepted FS exists; no long-lived source key is printed |
| 7 | writer | edit and commit | commit succeeds through accepted access |
| 8 | owner | revoke or expire another share, then writer tries accept | revoked or expired share cannot be accepted |

This proves a grant can become usable access through the service control plane.

#### 27. AgentFS Events Replay And Watch

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | observer | start `watch <owner-root> --agentfs --json` | process waits for AgentFS events |
| 2 | writer | commit a file | `commit.accepted` and `commit.materialized` occur |
| 3 | observer | read watch output | events include `stream: agentfs`, `seq`, `fs_id`, actor, subject |
| 4 | observer | stop watch, then run `fs events <fs> --after <seq>` | replay resumes after last seen event |

This proves a passive Agent can observe changes without reading metadata files.

#### 28. Authorizing Grant Audit

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | grant writer to `writer` | grant id is returned |
| 2 | writer | commit a file | commit succeeds |
| 3 | observer | `fs events <fs> --json` | `commit.accepted.data.authorized_by.grant_id` equals the grant id |
| 4 | owner | revoke the writer grant | grant is no longer active |
| 5 | observer | replay old commit event and inspect commit metadata oracle | old audit still points to original grant id |
| 6 | owner | commit a file | audit shows `authorized_by.type = owner` |

This proves every accepted mutation explains which authority allowed it.

#### 29. FS Status As Agent Decision Surface

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | create FS and grant reader, writer, manager | roles exist |
| 2 | each agent | `fs status <root-or-fs> --json` | output includes agent id, role, capabilities, head, base, materialization state |
| 3 | writer | edit local file | status reports dirty count |
| 4 | writer | become stale after another commit | status reports stale |
| 5 | owner | put FS into failed materialization state | status reports non-ready state and warning |

This proves `fs status` is enough for an Agent to decide whether it can act.

#### 30. Post-Event Hook Automation

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | `hooks add project --event commit.accepted -- <script>` | hook record is stored |
| 2 | writer | commit a file | commit succeeds normally |
| 3 | hook script | write received JSON to `hook-output/` | JSON includes event kind, fs id, commit id, actor |
| 4 | observer | `fs events` | optional hook success or hook failure event is visible |
| 5 | reader | try to add hook | `grant_denied` |

This proves hooks can automate work from AgentFS events and require manage
authority.

#### 31. Blocking Preflight Hook

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | add `commit.preflight` hook with `blocking: true` | hook is active |
| 2 | writer | commit content that makes hook return non-zero | commit is rejected before head advances |
| 3 | test | inspect remote/head | no shared truth mutation occurred |
| 4 | writer | commit content that hook accepts | commit succeeds |
| 5 | observer | `fs events` | hook result is visible in event data or hook event |

This proves blocking hooks can allow or block commit acceptance.

#### 32. `AGENTS.md` Rule Enforcement

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | manager | commit `AGENTS.md` with `required_checks` and `protected_paths` machine block | rules become active |
| 2 | writer | commit normal path with required check passing | commit succeeds |
| 3 | writer | commit normal path with required check failing | commit rejected before head advances |
| 4 | writer | commit protected path without manage authority | `grant_denied` or policy error |
| 5 | manager | commit protected path | commit succeeds |
| 6 | manager | commit invalid `AGENTS.md` machine block | rejected with `agent_rules_invalid`; previous rules remain active |

This proves FS-local rules affect commit decisions only through defined
machine-readable rules.

#### 33. Proposal And Approval Commit

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | grant contributor to `contributor` | contributor has `read` and `propose`, not `commit` |
| 2 | contributor | edit and run `commit apply` | `grant_denied` |
| 3 | contributor | `commit propose <root> --message "change"` | proposal id returned; head unchanged |
| 4 | manager-only | inspect proposal and try accept | accept is rejected; head unchanged |
| 5 | owner | inspect proposal and accept | proposal becomes accepted commit; head advances |
| 6 | writer-b | advance head before another proposal accept | stale proposal accept is rejected |

This proves proposal/approval is a separate path from direct commit.

#### 34. Path-Scoped Grants

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | grant writer to `writer` with scope `docs/**` | writer has scoped commit authority |
| 2 | writer | edit `docs/a.md` and commit | commit succeeds |
| 3 | writer | edit `src/a.rs` and commit | rejected with path-scope policy error |
| 4 | writer | edit both `docs/b.md` and `src/b.rs` in one commit | entire commit is rejected |
| 5 | observer | inspect accepted docs commit audit | `authorized_by` includes grant id and matched path scope |

This proves path scopes restrict which local changes can become shared truth.

### AgentFS Product-Complete E2E Gaps

The E2E definition above is broader than the current implementation.

Before calling the AgentFS core product complete, the suite must close the P0
and P1 gaps:

| Gap | Why It Matters | Current Status |
| --- | --- | --- |
| Server-side share and accept | A grant should be usable from the grantee perspective after login, not only from the owner's local setup. | 已有第一版：`sectiond serve` 提供 HTTP Control Service；客户端只配置 endpoint，不配置 SourceProfile 或 backing source；`fs share`、`fs available`、`fs accept` 走 Control Service；pending share 在 backing grant 被 revoke 后不能再 available/accept；`fs accept` 返回带过期时间的 service-issued credential binding；`fs attach`、`commit status`、`commit apply` 在访问 backing source 前都会刷新 service-issued credential；`fs list/status/events/watch` 只对有 read capability 的 agent 暴露。后续生产硬化项是传输认证和高并发部署。 |
| AgentFS watch/replay | Observable mutation is part of the product contract. Reading metadata files is only a secondary oracle. | 已有第一版：`fs events` 支持 replay/after，`watch --agentfs` 输出 AgentFS 事件，事件有递增 `seq`，且需要 read capability；grant/revoke 事件以 Control Service 为权威，并和 backing event mirror 合并输出。后续生产硬化项是多副本服务端的并发顺序保障。 |
| Authorizing grant audit | Agents should know which grant or owner authority allowed a mutation. | 已有第一版：commit record 和 `commit.accepted` event 记录 `authorized_by`，E2E 已通过 `fs events` 验证。 |
| Trustworthy dirty base | Commit correctness depends on comparing against the mounted base, not just current remote state. | 已有第一版：本地 store 记录 mount/base，E2E 验证篡改 `.section/root.json` 不能绕过 stale-base；外部直接修改 backing source 时，commit 返回 `remote_drift`，不会写 accepted commit。更完整的 tree diff 证明可作为后续增强。 |
| Commit/materialization snapshot match | Accepted commit metadata must describe the bytes that became truth. | 已有第一版：`commit apply` 先写 staging snapshot，commit metadata 记录 snapshot，物化从 snapshot 读取。E2E 验证 live root 后续修改仍是 dirty work；本地 marker 更新失败只作为 warning 返回，不推翻已完成的治理真相。 |
| Materialization failure and repair | Contract says accepted failed commits block later commits and can be repaired. | 已有第一版：failed head 阻塞后续 commit，`commit repair` 用原 staging snapshot 修复同一个 commit，repair 后可以继续接受新 commit。 |
| Local root canonicalization and overlap | Attached root identity must survive path spelling and avoid nested-root leakage. | 已有第一版：attach 和 source bind 会存 canonical local root；E2E 验证 attach JSON、marker、status 都使用 canonical root；父子 root 和 backing root overlap 会被拒绝；同一 FS 在同一 local store 只有一个 active local root，reattach 会移动 root。 |
| Low-level source command guardrails | Source commands can bypass or damage AgentFS governance if treated as ordinary user operations. | 已有第一版 guardrails。MVP 明确不提供低层 force 绕过。 |
| Metadata schema, lock, and event immutability | Shared metadata is the governance record and must be robust across agents/backends. | 已有第一版：schema/version/ID/path validation、head/commit/event FS 一致性检查、初始化失败回滚、service event authority、event `seq`、event 文件用 backend `if_not_exists` 写入、active head lock 会阻塞 commit，commit accepted 事件失败不推进 head。后端必须支持条件创建和锁目录一致列举，不做 fallback。 |
| JSON error contract | Agent callers need stable machine-readable failures. | 已有第一版：`agent`、`fs`、`commit`、`watch --agentfs` 在 `--json` 失败时输出稳定 `error.code`、`retryable`、`details`。参数错误是 `invalid_arguments`，普通运行错误统一落到 `operation_failed`。 |
| File/dir replacement preflight | Materialization must not leave half-applied filesystem shape changes. | 已有第一版：同一路径文件/目录类型替换会在 accepted commit 写入前失败，返回 `path_type_conflict`；非空目录删除会物化为干净的远端删除。 |
| FS status decision surface | Agents need one command to know whether they can act. | 已有第一版：`fs status --json` 支持本地 path，输出 role、capabilities、head、base、dirty、stale、materialization、warnings、next_actions。 |

### Future Feature E2E Gates

P2 features are extension gates. They should not block the AgentFS core product
complete call, but each feature must pass its own E2E before it is exposed as
implemented.

| Feature | Why It Matters | Current Status |
| --- | --- | --- |
| Hooks | Automation should run from AgentFS events and optionally gate commits. | Design defined: post-event hooks and blocking preflight hooks; implementation open. |
| `AGENTS.md` rules | FS-local rules should affect Agent behavior through explicit machine-readable policy. | Design defined: minimal machine block, protected paths, required checks; implementation open. |
| Proposal/approval | Some collaborators should propose changes without direct commit authority. | Design defined: `propose` capability, proposal lifecycle, accept/reject; implementation open. |
| Path-scoped grants | Coarse FS-wide writer grants may be too broad. | Design defined: allow-only path scopes on grants; implementation open. |

### Current CLI Product Test Implementation

The current CLI product tests live mainly in:

- `crates/section-cli/tests/agentfs_e2e.rs`
- `crates/section-cli/tests/agentfs_control_plane.rs`

It covers the product behaviors that are implemented today:

| Test | E2E Scenarios Covered |
| --- | --- |
| `e2e_writer_commit_becomes_shared_truth_for_owner` | owner creates FS; writer commit becomes shared truth; `fs accept` returns service-issued credential binding with expiry; `fs attach`、`commit status`、`commit apply` refresh service-issued credentials before backing-source access; owner observes accepted content; staging snapshot、repair、repair 后继续 commit、`fs events` replay、`watch --agentfs`、event `seq`、`authorized_by`、JSON error payload 都可验证 |
| `agentfs_control_plane::http_control_service_shares_without_client_source_profile_or_keys` | `sectiond serve` HTTP Control Service owns SourceProfile/backing source; owner/writer clients only configure endpoint; tampered writer auth token is rejected; restored writer discovers、accepts、attaches、commits without client-side source profile or backing-source keys |
| `e2e_fs_ref_resolves_source_name_and_rejects_ambiguity` | `fs status` resolves by source name; duplicate source-name refs fail with `ambiguous_fs_ref`; exact fs id still resolves |
| `e2e_bad_metadata_in_unrelated_source_does_not_block_fs_lookup` | unrelated source 里坏的 `.section/agentfs/fs.json` 不会阻塞健康 FS 的 `fs status` 和 `fs events` |
| `e2e_rejects_invalid_shared_metadata_schema_and_links` | corrupted commit schema, wrong head FS link, and wrong event FS link fail with `malformed_shared_metadata` |
| `e2e_grants_control_attach_manage_and_commit_authority` | reader denied commit but can status/events; ungranted agent denied attach/status/events; downgrade removes commit authority; manager can grant、share、revoke but cannot commit; `fs status` exposes role、capabilities、dirty、next_actions |
| `e2e_revoke_removes_commit_access_and_blocks_pending_share_accept` | `fs revoke` removes commit access from an attached writer, emits replayable `grant.revoked`, prevents owner grant revoke, and blocks pending share accept after backing grant revoke |
| `agentfs_control_plane::reader_cannot_commit_and_ungranted_agent_cannot_attach` | reader cannot commit; ungranted attach fails; low-level `source sync`、`source bind`、`source remove`、`write`、`path inspect` cannot mutate or inspect AgentFS-backed source as normal source/path |
| `e2e_grant_survives_backing_event_mirror_failure` | grant 的 backing event mirror 写失败时，Control Service 事件仍能通过 `fs events` 看到，writer 仍能 accept、attach、commit |
| `e2e_stale_writer_cannot_overwrite_new_truth` | stale writer cannot commit over a newer accepted head; 篡改本地 marker 不能绕过 trusted mount base；`fs status` exposes stale state and `sync` next action |
| `e2e_backing_source_drift_cannot_be_committed_over` | backing source 被外部直接修改后，commit 返回 `remote_drift`；head、backing file、accepted event 数量都不变 |
| `e2e_hardening_rejects_unsafe_backing_source_and_attach_root` | non-empty backing source rejected; backing root cannot be attached as working root |
| `e2e_create_failure_rolls_back_service_and_local_cache` | create metadata 初始化失败时，Control Service rows、本地 AgentFS source cache、远端 `.section` metadata 都会回滚；恢复后可以重新 create |
| `e2e_attach_canonicalizes_local_root_identity` | attach 使用带 `..` 的路径拼写时，返回 JSON、root marker、`fs status` 都记录 canonical local root |
| `e2e_fs_status_reports_corrupt_local_marker` | corrupt `.section/root.json` is reported as `malformed_shared_metadata` by `fs status <local path> --json` instead of being swallowed as `unknown_fs` |
| `e2e_section_directory_is_not_committed_as_user_content` | local `.section/**` files are ignored by commit dirty detection and do not become shared user content |
| `e2e_commit_preflight_rejects_empty_message_and_empty_commit` | empty commit message and empty dirty set fail before head advances or new accepted event is created |
| `e2e_reattach_moves_single_local_root` | reattaching the same FS moves the active local root, removes the old marker, and status resolves only from the new root |
| `e2e_rejects_file_dir_type_replacement_before_acceptance` | file/dir type replacement is rejected before a new accepted commit; head and backing source remain unchanged |
| `e2e_non_empty_directory_delete_materializes_cleanly` | non-empty directory delete commits as shared truth; remote subtree is removed and working copy becomes clean |
| `e2e_materialization_failure_emits_fs_error_event` | real backing-source materialization failure leaves failed head, emits `commit.materialization_failed` and `fs.error`, and keeps unwritten file out of backing source |
| `e2e_event_write_failure_does_not_advance_commit_head` | `commit.accepted` event 写失败时，commit 命令失败；head 不前进，用户文件不物化 |
| `e2e_rejects_non_utf8_commit_paths` | 非 UTF-8 本地文件名不会被 lossy 转成共享路径；commit 返回 `non_utf8_path`，head 和 accepted event 不变 |
| `e2e_commit_success_survives_local_marker_update_failure` | accepted/materialized commit still succeeds when final local marker update fails; warning is returned and trusted mount base is updated |
| `e2e_hardening_rejects_symlink_commit_paths` | symlink paths cannot materialize files outside the working root |

It intentionally does not mark the core product-complete gaps above as
passing tests until those product capabilities exist.

### Current MVP E2E Non-Goals

Until the future feature E2Es above are implemented, the current MVP E2E suite
must not claim unsupported behavior:

- no path-scoped grant enforcement,
- no hook execution,
- no `AGENTS.md` enforcement,
- no low-level force bypass mode for AgentFS-backed sources.

Low-level source/path tests remain regression tests for the sync substrate. They
are not proof of AgentFS governance.

## Regression Tests

Run existing tests to prove AgentFS does not break source/path infrastructure:

```bash
cargo test -p section-provider
cargo test -p sectiond
cargo test -p section-cli --test path_control_plane
cargo test -p section-cli --test sync_control_plane
cargo test -p section-cli --tests
```

## MVP Out-Of-Scope Assertions

The MVP must not imply unsupported behavior:

| Surface | Assertion |
| --- | --- |
| Hooks | event names may be reserved, hook execution is not implemented |
| `AGENTS.md` | file is synced as normal content, Markdown rules are not enforced |
| path-scoped grants | all grants are FS-wide |
| proposals/approvals | direct commit is the only MVP mutation path |

## 完成标准

AgentFS 核心完整完成，必须同时满足：

- P0 和 P1 场景都有 E2E。
- 所有 E2E 都通过公开 CLI 跑通。
- owner、reader、writer、manager 的权限行为都有测试。
- share、available、accept、attach 的跨机业务路径有测试。
- commit 会检查权限、base、dirty、reserved path。
- commit 使用 staging snapshot，metadata 和物化文件一致。
- 本地 marker 更新失败不会让已 accepted/materialized 的 commit 被报告为失败。
- accepted commit 记录 `authorized_by`。
- materialization 失败会阻塞后续 commit。
- `commit repair` 修复同一个 commit。
- `fs status --json` 能告诉 Agent 当前能不能行动。
- `fs events` 和 `watch --agentfs` 能让 Agent 观察变化。
- 每个公开 AgentFS JSON 错误都有稳定 `error.code`、`retryable`、`details`。
- 低层 `source/path` 命令不能绕过 AgentFS。
- 现有 source/path 回归测试仍然通过。
- 未实现的 P2 功能不能暴露成可用功能。
- 设计文档、测试计划、CLI 行为一致。

### 核心完成标准覆盖审计

| 完成标准 | 当前证据 | 状态 |
| --- | --- | --- |
| P0 和 P1 场景都有 E2E | `agentfs_e2e.rs` 覆盖 owner/writer/reader/manager、share/accept/attach、commit、events/status、failure/repair、metadata lock、guardrails；`agentfs_control_plane.rs` 覆盖服务端 source profile/credential/cache 和低层 source/path 拒绝 | covered |
| 所有用户行为 E2E 都通过公开 CLI 跑通 | E2E 使用 `run_section` 执行 `section` CLI；测试 harness 只负责启动 HTTP Control Service 或制造故障 | covered |
| owner、reader、writer、manager 权限行为都有测试 | `e2e_grants_control_attach_manage_and_commit_authority`，并覆盖 manager grant/share/revoke、owner grant 不可 revoke | covered |
| share、available、accept、attach 的跨机业务路径有测试 | `http_control_service_shares_without_client_source_profile_or_keys` 证明客户端只配置 endpoint 也能 share/available/accept/attach/commit；`e2e_writer_commit_becomes_shared_truth_for_owner`、`e2e_revoke_removes_commit_access_and_blocks_pending_share_accept`、`writer_share_accept_attach_commit_and_owner_observes_truth`、`client_seed_cannot_overwrite_existing_source_profile` 覆盖本地 harness 和 credential refresh | covered |
| commit 检查权限、base、dirty、reserved path | 权限：`e2e_grants_control_attach_manage_and_commit_authority`；base：`e2e_stale_writer_cannot_overwrite_new_truth`；dirty：`e2e_commit_preflight_rejects_empty_message_and_empty_commit`；reserved path：`e2e_section_directory_is_not_committed_as_user_content` | covered |
| commit 使用 staging snapshot，metadata 和物化文件一致 | `e2e_writer_commit_becomes_shared_truth_for_owner` 检查 staging manifest；同一测试里 live root 后续修改保持 dirty work，repair 用原 snapshot 物化原内容 | covered |
| 本地 marker 更新失败不会让已 accepted/materialized 的 commit 被报告为失败 | `e2e_commit_success_survives_local_marker_update_failure` | covered |
| accepted commit 记录 `authorized_by` | `e2e_writer_commit_becomes_shared_truth_for_owner` 和 `writer_share_accept_attach_commit_and_owner_observes_truth` 检查 commit/event 的 grant authority | covered |
| materialization 失败会阻塞后续 commit | `e2e_materialization_failure_emits_fs_error_event` 和 `e2e_writer_commit_becomes_shared_truth_for_owner` 的 failed-head block | covered |
| `commit repair` 修复同一个 commit | `e2e_writer_commit_becomes_shared_truth_for_owner` 检查 repair 返回同一 `commit_id`、恢复原 staging 内容，并且 repair 后新 commit 可以继续 | covered |
| `fs status --json` 能告诉 Agent 当前能不能行动 | `e2e_grants_control_attach_manage_and_commit_authority`、`e2e_stale_writer_cannot_overwrite_new_truth`、`e2e_fs_status_reports_corrupt_local_marker` | covered |
| `fs events` 和 `watch --agentfs` 能让 Agent 观察变化 | `e2e_writer_commit_becomes_shared_truth_for_owner`、`e2e_grant_survives_backing_event_mirror_failure` | covered |
| 每个公开 AgentFS JSON 错误都有稳定 `error.code`、`retryable`、`details` | `assert_json_error` 在 E2E/control-plane tests 中统一检查 `code`、`retryable`、`details`；覆盖 `grant_denied`、`stale_base`、`remote_drift`、`materialization_failed`、`metadata_write_conflict`、`malformed_shared_metadata`、`non_utf8_path`、`unknown_fs`、`ambiguous_fs_ref`、`operation_failed` | covered |
| 低层 `source/path` 命令不能绕过 AgentFS | `reader_cannot_commit_and_ungranted_agent_cannot_attach`、`config_source_named_like_agentfs_source_cannot_bypass_file_router` | covered |
| 现有 source/path 回归测试仍然通过 | Regression Tests section defines `cargo test -p section-cli --tests`; verification record is kept in issue triage | covered |
| 未实现的 P2 功能不能暴露成可用功能 | MVP Out-Of-Scope Assertions keeps Hooks、`AGENTS.md` enforcement、path-scoped grants、proposals out of core completion | covered |
| 设计文档、测试计划、CLI 行为一致 | This audit is the source of truth for #46; inconsistent path diagnostic wording was removed | covered |

P2 功能单独完成。每个 P2 功能完成时，必须满足：

- 有自己的 E2E。
- 不破坏 AgentFS 核心 E2E。
- 错误也走统一 JSON 错误格式。
- 文档和实现一致。
