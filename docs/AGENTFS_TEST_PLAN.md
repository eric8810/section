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

Sharing tests must run a Section Control Service test harness. The harness owns
agent identity, installation identity, grants, shares, source profiles, and
short-lived sync credentials.

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
| owner creates FS | agent logged in, source profile exists, `fs create project` | FS exists, source profile is bound, owner grant exists, head exists with null commit |
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
- run a Section Control Service test harness for sharing and credential tests,
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
| 2 | writer | inspect user-visible local root | `.section/agentfs/**` is not materialized as ordinary content |
| 3 | writer | create normal files and commit | commit paths exclude AgentFS metadata |
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
| 3 | writer | commit symlink path | rejected; target outside root does not materialize |
| 4 | writer | inspect `fs attach --json` | source options and credentials are not exposed |

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
| 3 | writer | `commit status` or `commit apply` from old local root | drift/conflict/stale state is surfaced |
| 4 | test | inspect head and commit records | no accepted commit silently blesses external drift |

This proves external backing-source changes are not governed commits. The exact
CLI error may evolve, but the E2E must prove the mutation is not silently
accepted as AgentFS truth.

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

#### 13. Materialization Failure Blocks New Commits

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | writer | commit a dirty path while backing source fails materialization | commit remains accepted with `failed_to_materialize` |
| 2 | writer | attempt another commit | command fails before accepting a new commit |
| 3 | owner or writer | inspect status | FS reports non-ready materialization state |
| 4 | test | inspect head | head remains on the failed accepted commit |

This proves governance truth can advance independently from file
materialization, and that the system does not accept follow-up mutations while
the head is not materialized.

#### 14. Granted Agent Bootstrap Is Product-Visible

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

#### 15. Local Root Identity Is Stable

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
- Section Control Service harness，用来测试 share、discovery、grant、credential。
- 临时 hook script，用来记录输入 JSON、返回成功或失败。

#### 16. Trusted Dirty Base And Status

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

#### 17. Commit Snapshot Isolation

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | writer | edit `docs/note.txt` to version A | `commit status` reports version A dirty |
| 2 | writer | start `commit apply` with a paused materialization provider | staging snapshot is created before metadata write |
| 3 | test harness | change live local file to version B before materialization resumes | live root now differs from staging |
| 4 | harness | resume materialization | remote file contains version A |
| 5 | test | inspect commit record and `fs status` | commit path hash matches version A; version B remains local dirty work |

This proves accepted commit metadata describes the bytes that became shared
truth.

#### 18. Materialization Repair

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | writer | commit with provider configured to fail materialization once | commit is accepted; head points to it; state is `failed_to_materialize` |
| 2 | writer | run another `commit apply` | rejected with `materialization_failed` |
| 3 | owner | `fs status --json` | non-ready materialization state is visible |
| 4 | writer | `commit repair <root> --commit <commit_id>` after provider recovers | same commit id becomes `materialized` |
| 5 | writer | run a new commit | new commit can proceed after repair |

This proves repair fixes the accepted head instead of creating a second truth.

#### 19. Metadata Validation And Bad Data Isolation

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | create valid FS `good` | status succeeds |
| 2 | test harness | create another source with malformed `.section/agentfs/fs.json` | corrupt source exists |
| 3 | owner | `fs status good --json` | succeeds; may include warning about unrelated bad metadata |
| 4 | owner | `fs status corrupt --json` | fails with `malformed_shared_metadata` |
| 5 | test harness | create two FS records with same display name | lookup by name fails with `ambiguous_fs_ref` |
| 6 | owner | lookup by exact `fs_id` | exact id still resolves |

This proves bad metadata is isolated to the affected FS or lookup candidate.

#### 20. Event Immutability And Sequence Replay

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | create FS, grant writer, writer commits twice | multiple AgentFS events exist |
| 2 | observer | `fs events <fs> --json` | events have strictly increasing `seq` |
| 3 | observer | `fs events <fs> --after <seq>` | replay returns only later events |
| 4 | test harness | try to pre-create or overwrite an existing event id before a command writes | command fails or creates a different event; old event is unchanged |
| 5 | observer | replay again | event order is still by `seq`, not filesystem list order |

This proves events are append-only and replayable.

#### 21. Source And Path Guardrails

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | create AgentFS-backed source | source record is marked as AgentFS-owned |
| 2 | owner | run `source sync` on that source | rejected with AgentFS guardrail error |
| 3 | owner | run `source remove` or `source bind` | rejected with AgentFS guardrail error |
| 4 | owner | try any low-level force path | rejected; MVP has no low-level bypass |
| 5 | owner | run `path inspect/compare/resolve` under the root | command works as diagnostic and warns it is not the governance surface |

This proves low-level source/path commands do not silently bypass AgentFS.

#### 22. JSON Error Contract

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | stranger | `fs attach project stranger-root --json` | `error.code = grant_denied`, `retryable = false` |
| 2 | writer | stale `commit apply --json` | `error.code = stale_base`, `retryable = true` |
| 3 | owner | status corrupt metadata with `--json` | `error.code = malformed_shared_metadata` |
| 4 | writer | commit while head failed materialization with `--json` | `error.code = materialization_failed`, `retryable = true` |
| 5 | test harness | hold metadata lock, then run grant or commit with `--json` | `error.code = metadata_write_conflict`, `retryable = true` |

This proves Agent callers can make decisions from stable error codes.

#### 23. File/Dir Replacement Preflight

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | writer | commit file `docs/api` | file materializes |
| 2 | writer | replace `docs/api` with directory `docs/api/index.md` | commit succeeds; remote has directory and child |
| 3 | writer | replace directory `docs/api` with file `docs/api` | commit succeeds; remote has file only |
| 4 | harness | repeat with provider configured to reject required delete/create plan | commit fails before metadata write |
| 5 | test | inspect head and remote after failure | no half-materialized accepted commit |

This proves commit acceptance knows whether materialization can execute the path
plan safely.

#### 24. Local Root Identity And Overlap

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | writer | attach using `/tmp/project/../project` | mount stores canonical root |
| 2 | writer | run `fs status /tmp/project` | resolves the same `mount_id` |
| 3 | writer | attach child root under existing root | rejected |
| 4 | writer | attach parent root over existing root | rejected |
| 5 | writer | attach backing source root as working root | rejected |

This proves root identity is stable and nested roots cannot leak control files.

#### 25. Server Share And Accept

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

#### 26. AgentFS Events Replay And Watch

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | observer | start `watch <owner-root> --agentfs --json` | process waits for AgentFS events |
| 2 | writer | commit a file | `commit.accepted` and `commit.materialized` occur |
| 3 | observer | read watch output | events include `stream: agentfs`, `seq`, `fs_id`, actor, subject |
| 4 | observer | stop watch, then run `fs events <fs> --after <seq>` | replay resumes after last seen event |

This proves a passive Agent can observe changes without reading metadata files.

#### 27. Authorizing Grant Audit

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | grant writer to `writer` | grant id is returned |
| 2 | writer | commit a file | commit succeeds |
| 3 | observer | `fs events <fs> --json` | `commit.accepted.data.authorized_by.grant_id` equals the grant id |
| 4 | owner | revoke the writer grant | grant is no longer active |
| 5 | observer | replay old commit event and inspect commit metadata oracle | old audit still points to original grant id |
| 6 | owner | commit a file | audit shows `authorized_by.type = owner` |

This proves every accepted mutation explains which authority allowed it.

#### 28. FS Status As Agent Decision Surface

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | create FS and grant reader, writer, manager | roles exist |
| 2 | each agent | `fs status <root-or-fs> --json` | output includes agent id, role, capabilities, head, base, materialization state |
| 3 | writer | edit local file | status reports dirty count |
| 4 | writer | become stale after another commit | status reports stale |
| 5 | owner | put FS into failed materialization state | status reports non-ready state and warning |

This proves `fs status` is enough for an Agent to decide whether it can act.

#### 29. Post-Event Hook Automation

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | `hooks add project --event commit.accepted -- <script>` | hook record is stored |
| 2 | writer | commit a file | commit succeeds normally |
| 3 | hook script | write received JSON to `hook-output/` | JSON includes event kind, fs id, commit id, actor |
| 4 | observer | `fs events` | optional hook success or hook failure event is visible |
| 5 | reader | try to add hook | `grant_denied` |

This proves hooks can automate work from AgentFS events and require manage
authority.

#### 30. Blocking Preflight Hook

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | add `commit.preflight` hook with `blocking: true` | hook is active |
| 2 | writer | commit content that makes hook return non-zero | commit is rejected before head advances |
| 3 | test | inspect remote/head | no shared truth mutation occurred |
| 4 | writer | commit content that hook accepts | commit succeeds |
| 5 | observer | `fs events` | hook result is visible in event data or hook event |

This proves blocking hooks can allow or block commit acceptance.

#### 31. `AGENTS.md` Rule Enforcement

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

#### 32. Proposal And Approval Commit

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | grant contributor to `contributor` | contributor has `read` and `propose`, not `commit` |
| 2 | contributor | edit and run `commit apply` | `grant_denied` |
| 3 | contributor | `commit propose <root> --message "change"` | proposal id returned; head unchanged |
| 4 | manager-only | inspect proposal and try accept | accept is rejected; head unchanged |
| 5 | owner | inspect proposal and accept | proposal becomes accepted commit; head advances |
| 6 | writer-b | advance head before another proposal accept | stale proposal accept is rejected |

This proves proposal/approval is a separate path from direct commit.

#### 33. Path-Scoped Grants

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
| Server-side share and accept | A grant should be usable from the grantee perspective after login, not only from the owner's local setup. | 已有第一版：`fs share`、`fs available`、`fs accept` 走 Control Service harness。还缺生产服务端、真实 credential TTL 和刷新。 |
| AgentFS watch/replay | Observable mutation is part of the product contract. Reading metadata files is only a secondary oracle. | 已有第一版：`fs events` 支持 replay/after，`watch --agentfs` 输出 AgentFS 事件，事件有递增 `seq`。还缺更强的服务端事件 API 和并发顺序保障。 |
| Authorizing grant audit | Agents should know which grant or owner authority allowed a mutation. | 已有第一版：commit record 和 `commit.accepted` event 记录 `authorized_by`，E2E 已通过 `fs events` 验证。 |
| Trustworthy dirty base | Commit correctness depends on comparing against the mounted base, not just current remote state. | 部分实现：本地 store 记录 mount/base。还缺 `base_manifest_hash` 和严格 tree diff。 |
| Commit/materialization snapshot match | Accepted commit metadata must describe the bytes that became truth. | 已有第一版：`commit apply` 先写 staging snapshot，commit metadata 记录 snapshot，物化从 snapshot 读取。E2E 验证 live root 后续修改仍是 dirty work。 |
| Materialization failure and repair | Contract says accepted failed commits block later commits and can be repaired. | 已有第一版：failed head 阻塞后续 commit，`commit repair` 用原 staging snapshot 修复同一个 commit。 |
| Local root canonicalization and overlap | Attached root identity must survive path spelling and avoid nested-root leakage. | 部分实现：已有 backing root、父子 root、runtime guardrails。还缺持久 `mount_id`/canonical root 完整断言。 |
| Low-level source command guardrails | Source commands can bypass or damage AgentFS governance if treated as ordinary user operations. | 已有第一版 guardrails。MVP 明确不提供低层 force 绕过。 |
| Metadata schema and event immutability | Shared metadata is the governance record and must be robust across agents/backends. | 部分实现。已有 event `seq` 和事件路径覆盖拒绝；还缺统一 schema validation、坏数据隔离、backend create-if-absent 断言。 |
| JSON error contract | Agent callers need stable machine-readable failures. | 部分实现。还缺所有公开 AgentFS 命令统一 `error.code`、`retryable`、`details`。 |
| File/dir replacement preflight | Materialization must not leave half-applied filesystem shape changes. | 未实现。设计已固定：metadata 写入前生成 path operation plan。 |
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

### Current E2E Implementation

The current CLI E2E implementation lives in
`crates/section-cli/tests/agentfs_e2e.rs`.

It covers the product behaviors that are implemented today:

| Test | E2E Scenarios Covered |
| --- | --- |
| `e2e_writer_commit_becomes_shared_truth_for_owner` | owner creates FS; writer commit becomes shared truth; owner observes accepted content; staging snapshot、repair、`fs events` replay、`watch --agentfs`、event `seq`、`authorized_by` 都可验证 |
| `e2e_grants_control_attach_manage_and_commit_authority` | reader denied commit; ungranted agent denied attach; downgrade removes commit authority; manager can manage but cannot commit; `fs status` exposes role、capabilities、dirty、next_actions |
| `e2e_stale_writer_cannot_overwrite_new_truth` | stale writer cannot commit over a newer accepted head; `fs status` exposes stale state and `sync` next action |
| `e2e_hardening_rejects_unsafe_backing_source_and_attach_root` | non-empty backing source rejected; backing root cannot be attached as working root |
| `e2e_hardening_rejects_symlink_commit_paths` | symlink paths cannot materialize files outside the working root |

It intentionally does not mark the core product-complete gaps above as
passing tests until those product capabilities exist.

### Current MVP E2E Non-Goals

Until the future feature E2Es above are implemented, the current MVP E2E suite
must not claim unsupported behavior:

- no path-scoped grant enforcement,
- no hook execution,
- no `AGENTS.md` enforcement,
- no guarantee that low-level `source` commands preserve governance.

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

P2 功能单独完成。每个 P2 功能完成时，必须满足：

- 有自己的 E2E。
- 不破坏 AgentFS 核心 E2E。
- 错误也走统一 JSON 错误格式。
- 文档和实现一致。
