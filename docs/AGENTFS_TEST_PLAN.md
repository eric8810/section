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
- use separate config files and `data_dir` values for each agent,
- use one shared backing source for the FS,
- use separate local roots for each attached agent,
- perform user work through ordinary file operations under local roots,
- avoid calling `sectiond` Rust APIs directly.

E2E tests may inspect shared metadata files under `.section/agentfs/` as an
oracle, but the primary assertions should be made through CLI output and
observable filesystem state.

Until AgentFS event replay/watch is implemented, E2E tests may verify events by
reading `.section/agentfs/events/`. The final E2E gate must replace that fallback
with `section watch` or an AgentFS event replay command.

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
| 1 | owner | `agent register owner` | persisted `agt_` identity |
| 2 | owner | `fs create project --provider fs --opt root=remote` | `fs_` created; owner grant exists |
| 3 | owner | `fs list` and `fs status project` | owner can discover and inspect FS |
| 4 | owner | inspect `remote/.section/agentfs/` | `fs.json`, head, owner grant, `fs.created` event exist |

This proves agent-owned filesystem creation and initial governance metadata.

#### 2. Writer Commit Becomes Shared Truth

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | owner | `fs attach project owner-root` | owner root marker records null base head |
| 2 | writer | `agent register writer` | distinct `agt_` identity |
| 3 | owner | `fs grant project <writer_id> --role writer` | writer has `read` and `commit` capability |
| 4 | writer | add backing source for `project` | writer can resolve the shared FS source |
| 5 | writer | `fs attach project writer-root` | writer root materializes current shared truth |
| 6 | writer | write `writer-root/docs/note.txt` | remote file does not exist yet |
| 7 | writer | `commit status writer-root` | dirty path reports `docs/note.txt` as create |
| 8 | writer | `commit apply writer-root --message "add note"` | commit id returned; materialization succeeds |
| 9 | test | inspect head and commit metadata | head points to commit; commit actor is writer |
| 10 | test | inspect remote file | `remote/docs/note.txt` exists with writer content |
| 11 | owner | attach a fresh root or sync existing root | owner observes writer's accepted content |

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
| 1 | stranger | register and add backing source | local identity exists |
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
| 2 | writer | discover or accept access to the FS | writer can learn enough to attach without out-of-band source setup |
| 3 | writer | attach | attach succeeds using granted access path |
| 4 | test | inspect writer local store | source/bootstrap metadata is present without leaking credentials unnecessarily |

This proves grants lead to usable access from the grantee's perspective. The
current implementation still requires manual backing source setup, so this is a
required product-complete E2E after invite/source bootstrap/discovery is
designed.

#### 15. Local Root Identity Is Stable

| Step | Actor | Command or action | Expected |
| --- | --- | --- | --- |
| 1 | writer | attach using one spelling of a path | root marker and local binding are written |
| 2 | writer | run commit/status using an equivalent canonical path | command resolves the same attached root |
| 3 | writer | try overlapping parent/child roots | attach or bind rejects ambiguous overlap |
| 4 | test | inspect local bindings | one FS cannot accidentally treat another root marker as user content |

This proves local root identity does not depend on fragile path spelling or
overlapping directory layouts.

### Deferred E2E Scenarios

These belong in the product E2E suite, but require implementation that is not
part of the current MVP slice.

| Scenario | Required Feature | Expected Product Proof |
| --- | --- | --- |
| AgentFS watch/replay from CLI | AgentFS event replay/watch integration | a passive agent sees `commit.accepted` without reading metadata files directly |
| Authorizing grant audit | commit/event records grant id or owner authority | observer can identify which grant allowed an accepted mutation |
| Materialization repair | retry materialization command | accepted failed commit is repaired with the same commit id |
| Granted agent bootstrap | invite/source bootstrap/discovery design | grantee can attach without manual backing source setup |
| Commit snapshot isolation | frozen commit snapshot or equivalent preflight guard | commit metadata cannot diverge from materialized bytes |
| Hook-gated commit | hooks trust model and execution | hook result can allow or block commit acceptance |
| `AGENTS.md` rule enforcement | FS-local rule parser and policy engine | FS-local rules affect commit decisions |
| Proposal/approval commit | proposal workflow | direct commit is replaced or augmented by approval policy |

### Product-Complete E2E Coverage Gaps

The E2E definition above is broader than the current implementation. Before
calling AgentFS product-complete, the suite must close these gaps:

| Gap | Why It Matters | Current Status |
| --- | --- | --- |
| Granted-agent bootstrap | A grant should be usable from the grantee perspective, not only from the owner's local setup. | Open design: invite token, source export, shared source URI, or discovery service. |
| AgentFS watch/replay | Observable mutation is part of the product contract. Reading metadata files is only a test fallback. | Open implementation: no AgentFS replay/watch command yet. |
| Authorizing grant audit | Agents should know which grant or owner authority allowed a mutation. | Open implementation: commit/event do not record grant id. |
| Trustworthy dirty base | Commit correctness depends on comparing against the mounted base, not just current remote state. | Open implementation issue: base snapshot/path sync state needs tightening. |
| Commit/materialization snapshot match | Accepted commit metadata must describe the bytes that became truth. | Open implementation issue: TOCTOU between commit record and sync materialization. |
| Materialization failure and repair | Contract says accepted failed commits block later commits and can be repaired. | Partial: failed heads block; retry/repair command is missing. |
| Local root canonicalization and overlap | Attached root identity must survive path spelling and avoid nested-root leakage. | Partial: exact collision and backing-root overlap are guarded; canonical/overlap semantics remain open. |
| Low-level source command guardrails | Source commands can bypass or damage AgentFS governance if treated as ordinary user operations. | Open product decision: official escape hatch or guarded management surface. |
| Metadata schema and event immutability | Shared metadata is the governance record and must be robust across agents/backends. | Partial: schema validation and backend-level append-only semantics remain open. |

### Current E2E Implementation

The current CLI E2E implementation lives in
`crates/section-cli/tests/agentfs_e2e.rs`.

It covers the product behaviors that are implemented today:

| Test | E2E Scenarios Covered |
| --- | --- |
| `e2e_writer_commit_becomes_shared_truth_for_owner` | owner creates FS; writer commit becomes shared truth; owner observes accepted content; events and commit metadata exist |
| `e2e_grants_control_attach_manage_and_commit_authority` | reader denied commit; ungranted agent denied attach; downgrade removes commit authority; manager can manage but cannot commit |
| `e2e_stale_writer_cannot_overwrite_new_truth` | stale writer cannot commit over a newer accepted head |
| `e2e_hardening_rejects_unsafe_backing_source_and_attach_root` | non-empty backing source rejected; backing root cannot be attached as working root |
| `e2e_hardening_rejects_symlink_commit_paths` | symlink paths cannot materialize files outside the working root |

It intentionally does not mark the product-complete coverage gaps above as
passing tests until those product capabilities exist.

### E2E Non-Goals

The E2E suite must not claim unsupported products:

- no Messages workflow,
- no AgentDB, AgentGit, or AgentOps behavior,
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
