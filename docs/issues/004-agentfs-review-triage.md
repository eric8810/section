# Issue #004: AgentFS Review Findings Triage

## Status

`draft`

## Context

This document records the independent AgentFS review findings collected after
the initial AgentFS MVP control-plane implementation.

It is a triage document, not a replacement for the AgentFS requirements or MVP
contract. The purpose is to decide which findings are real, which ones are
release-blocking, which ones need product decisions, and which ones have already
been addressed in the current local branch or working tree.

The reviewed implementation is centered on:

- `docs/AGENTFS_REQUIREMENTS.md`
- `docs/AGENTFS_DESIGN_PROPOSAL.md`
- `docs/AGENTFS_MVP_CONTRACT.md`
- `docs/AGENTFS_TEST_PLAN.md`
- `crates/sectiond/src/agentfs.rs`
- `crates/sectiond/src/control_plane.rs`
- `crates/section-cli/src/cmd/{agent,fs,commit}.rs`
- `crates/section-cli/tests/agentfs_control_plane.rs`

## Triage Labels

### Validity

| Label | Meaning |
| --- | --- |
| `confirmed` | The finding is a real mismatch with current code or contract. |
| `partial` | The finding is real, but current local changes mitigate part of it. |
| `decision` | The finding is real only if the current contract remains unchanged; product or contract needs a decision. |
| `covered` | The finding is materially covered by another issue, but kept for traceability. |

### Priority

| Priority | Meaning |
| --- | --- |
| `P0` | Blocks claiming AgentFS governance correctness. Fix before continuing broad AgentFS feature work. |
| `P1` | Required before the AgentFS MVP can be called complete. |
| `P2` | Hardening or contract cleanup; should not block first internal iteration. |
| `P3` | Documentation, test depth, or later polish. |

### Status

| Status | Meaning |
| --- | --- |
| `fixed-local` | Addressed in the current local branch or working tree; uncommitted fixes still need commit/review. |
| `open` | No implementation fix yet. |
| `partial` | Some mitigation exists, but the finding is not fully closed. |
| `decision-needed` | Requires a contract/product decision before implementation. |

## Executive Summary

The review surfaced 52 findings. They are not all equal.

The most important pattern is that AgentFS is trying to build governance on top
of an existing bidirectional source/path sync system. Any path where low-level
sync can move bytes without an accepted AgentFS commit weakens the core product
claim:

```text
local edit != shared truth
accepted commit == shared truth mutation
```

The highest-priority remaining work is therefore:

1. protect AgentFS governance from source/path escape hatches,
2. make commit freshness and dirty detection use a trustworthy mounted base,
3. make AgentFS events replayable and observable,
4. make shared metadata writes failure-safe enough for the MVP,
5. settle discovery/invite semantics for a granted second agent.

## Priority Buckets

### P0: Governance And Security Blockers

These must be handled before making stronger claims about AgentFS correctness.

| ID | Finding | Validity | Status |
| --- | --- | --- | --- |
| 3 / 49 | `watch` and replay cannot observe AgentFS events. | confirmed | fixed-local |
| 5 | Commit dirty detection compares local tree to current remote, not the mounted base/path sync state. | confirmed | fixed-local |
| 7 | A second granted agent cannot attach by grant alone; it still needs manual source setup. | confirmed | fixed-local |
| 12 | Commit record and actual materialization have a TOCTOU gap. | confirmed | fixed-local |
| 18 | `fs create` can create `head=null` over a non-empty backing source. | confirmed | fixed-local |
| 19 | Stale-base checks trust editable `.section/root.json`. | confirmed | fixed-local |
| 20 | Governance state can advance before event writes succeed. | confirmed | open |
| 23 | `fs attach --json` can expose backing source options and credentials. | confirmed | fixed-local |
| 34 | Local symlinks are followed and can expose files outside the working root. | confirmed | fixed-local |
| 42 | The `fs` provider can attach the backing root as the working root, collapsing the truth boundary. | confirmed | fixed-local |
| 44 | `commit status/apply` discovers a marker from the input path but uses the store's current root instead of marker `local_root`. | confirmed | fixed-local |

### P1: MVP Completion Work

These should be fixed before calling the AgentFS MVP complete.

| ID | Finding | Validity | Status |
| --- | --- | --- | --- |
| 2 | Metadata writes need a real head lock and conflict path. | partial | partial |
| 6 | `fs create` can partially mutate local source registry before remote metadata validation succeeds. | partial | partial |
| 8 | Multiple local roots per FS are not implemented; current store is still source-level single-root. | decision | decision-needed |
| 9 | AgentFS JSON error contract is incomplete and inconsistent. | confirmed | fixed-local |
| 10 | Shared metadata lacks schema/version/cross-record consistency validation. | confirmed | open |
| 13 | Materialization retry/repair does not exist. | confirmed | fixed-local |
| 14 | Event ordering can reverse within the same millisecond due to random suffix sorting. | confirmed | fixed-local |
| 15 | FS metadata initialization is not failure-safe. | confirmed | open |
| 16 | Non-UTF-8 local paths are lossy-converted instead of rejected. | confirmed | open |
| 26 | Accepted commits do not record which grant allowed the mutation. | confirmed | fixed-local |
| 29 | A committed/materialized mutation can be reported as failed if final local marker write fails. | confirmed | fixed-local |
| 39 | Local root identity is not canonicalized. | confirmed | open |
| 40 | A bad `fs.json` in an unrelated source can block lookup of a healthy FS. | confirmed | open |
| 43 | FS reference resolution does not support local source aliases and can pick the first name collision. | confirmed | fixed-local |
| 45 | File/directory type replacement is accepted before sync later reports conflict. | confirmed | fixed-local |
| 46 | AgentFS tests do not cover the test plan. | partial | partial |
| 47 | Overlapping local roots can make a parent FS treat a child FS marker as user content. | confirmed | open |
| 48 | Low-level `source bind/unbind/remove` can break AgentFS root markers. | confirmed | open |
| 51 | AgentFS event files are not protected as append-only/immutable records. | partial | partial |

### P2: Hardening And Contract Cleanup

These are real, but can follow the governance blockers if the behavior is kept
internal and clearly marked incomplete.

| ID | Finding | Validity | Status |
| --- | --- | --- | --- |
| 4 | Attach/status should reject or expose failed materialization head. | partial | fixed-local |
| 11 | `fs status` role can be stale when multiple active grants exist. | partial | fixed-local |
| 17 | Materialization failure does not emit `fs.error`. | confirmed | open |
| 21 | Failed attach can leave a half-bound working copy. | partial | fixed-local |
| 22 | `fs list/status` can expose FS metadata without checking read grant. | decision | decision-needed |
| 27 | `section fs status <attached local path>` does not resolve from a local path. | confirmed | fixed-local |
| 28 | `fs status` lacks dirty state and full materialization detail. | partial | fixed-local |
| 30 | Non-empty directory deletion can race parent/child deletes in sync transport. | confirmed | open |
| 32 | `commit.materialized` event lacks path/state details. | confirmed | open |
| 35 | `.section/**` filtering is broader than the written contract's `.section/agentfs/**` reservation. | decision | decision-needed |
| 50 | `fs status` swallows corrupt local root markers. | confirmed | open |

### Fixed In Current Local Branch

These findings are considered addressed by the current local branch or current
working tree. Uncommitted rows still need review and commit before they are part
of the recorded baseline.

| ID | Finding | Evidence / Fix Direction |
| --- | --- |
| 1 | `fs attach` could publish local files without accepted commit. | Attach now requires an empty target root and rolls back failed attach. |
| 4 | Attach/status did not handle failed materialization. | Attach rejects non-materialized head; status reports materialization state. |
| 11 | Status role could be wrong with replaced grants. | Grant replacement now revokes previous active grants for that agent. |
| 18 | Non-empty backing source can become head-null FS truth. | `fs_create` now requires an empty backing source before source registration. |
| 21 | Attach failure could leave binding/marker behind. | Attach now rolls back binding and marker on sync failure/conflict. |
| 23 | `fs attach --json` could leak source credentials. | Attach returns a sanitized source summary without provider options. |
| 24 | Grant/revoke did not validate target `agent_id`. | Grant/revoke now validate `agt_[0-9a-f]{32}`. |
| 25 | Grant downgrade left older capabilities active. | New grant revokes previous active grants for that agent. |
| 31 | Shared `fs.attached` event leaked local absolute path. | Attach event data no longer includes `local_root`. |
| 33 | Reattach to a new empty root could delete remote content via old sync state. | Attach clears source sync state before initial materialization. |
| 34 | Symlinks could expose files outside the working root. | Commit path collection now rejects symlink entries instead of following them. |
| 36 | Attach did not check sync conflicts. | Attach rejects conflict result and rolls back. |
| 37 | Same local root could be silently stolen by another source. | Store rejects cross-source local-root collision. |
| 38 | `fs create` could overwrite an existing store source. | `fs_create` rejects existing source names. |
| 41 | Manager could self-grant writer. | Manager without commit capability cannot grant commit access to itself. |
| 42 | Local `fs` backing root could be used as working root. | Attach now rejects working roots that overlap the backing root for the local `fs` provider. |
| 44 | Commit commands ignored marker `local_root`. | Commit status/apply now fail if marker `local_root` differs from the current store binding. |
| 52 | Grant `capabilities` field was ignored by enforcement. | Capability checks now read the stored `capabilities` field. |

### Partially Addressed In Current Local Branch

These findings have useful mitigation, but remain open as design or backend
correctness work.

| ID | Finding | Remaining Gap |
| --- | --- | --- |
| 6 | `fs create` can partially mutate source registry before remote metadata validation succeeds. | Source registration now happens after remote preflight and is rolled back on initialization failure; partial remote metadata still needs initialization recovery. |
| 51 | AgentFS event files are not immutable. | Event writes now reject an already-existing event path, but this is not backend-level create-if-absent/CAS semantics. |

## Full Finding Ledger

This table preserves every reviewer finding for traceability.

| ID | Finding | Priority | Validity | Status | Recommended Action |
| --- | --- | --- | --- | --- | --- |
| 1 | `fs attach` can publish local-only files without accepted commit. | P0 | confirmed | fixed-local | Keep empty-root attach rule; add lower-level source escape protections. |
| 2 | AgentFS metadata writes have no required head lock. | P1 | partial | partial | Current lock is conservative; add create-if-absent/CAS semantics or document backend limits. |
| 3 | `watch` cannot observe AgentFS events. | P0 | confirmed | fixed-local | `fs events` and `watch --agentfs` now expose AgentFS events with `seq`. |
| 4 | Attach/status do not enforce or expose failed materialization. | P2 | partial | fixed-local | Keep tests for attach rejection and status output. |
| 5 | Commit dirty detection uses current remote, not mounted base/path sync state. | P0 | confirmed | fixed-local | Commit now checks current backing source against trusted path sync base first; external drift returns `remote_drift` before acceptance. |
| 6 | `fs create` mutates source registry before validating remote metadata. | P1 | partial | partial | Remote preflight and source rollback are in place; add staged initialization/recovery for partial remote metadata. |
| 7 | Granted second agent cannot attach by grant alone. | P0 | confirmed | fixed-local | Keep service-backed `fs share`, `fs available`, `fs accept`, and attach tests. |
| 8 | Multiple local roots per FS are not supported. | P1 | decision | decision-needed | Decide whether MVP really requires same-local-store multi-root; otherwise update contract. |
| 9 | JSON error contract is incomplete. | P1 | confirmed | fixed-local | AgentFS CLI JSON errors now include stable `code`, `retryable`, and `details`; argument failures use `invalid_arguments`, generic runtime failures use `operation_failed`. |
| 10 | Shared metadata lacks schema/version/consistency validation. | P1 | confirmed | open | Add validation traits for every metadata record. |
| 11 | `fs status` role can be wrong with multiple grants. | P2 | partial | fixed-local | Keep grant replacement tests; handle legacy duplicate active grants if needed. |
| 12 | Commit record can differ from actual materialized bytes. | P0 | confirmed | fixed-local | Commit now stages dirty paths first and materializes from the staging snapshot. |
| 13 | Materialization retry/repair is missing. | P1 | confirmed | fixed-local | `commit repair` reuses the original commit id and staging snapshot. |
| 14 | Event ordering can reverse within the same millisecond. | P1 | confirmed | fixed-local | Events now carry monotonic per-FS `seq` and replay sorts by `seq`. |
| 15 | FS metadata initialization is not failure-safe. | P1 | confirmed | open | Stage metadata or add initialization state and recovery. |
| 16 | Non-UTF-8 paths are lossy converted. | P1 | confirmed | open | Reject non-UTF-8 paths with typed error. |
| 17 | Materialization failure does not emit `fs.error`. | P2 | confirmed | open | Emit both commit failure and FS state event. |
| 18 | Non-empty backing source can become head-null FS truth. | P0 | confirmed | fixed-local | Reject non-empty backing source or import it as initial commit. |
| 19 | Stale-base check trusts editable root marker. | P0 | confirmed | fixed-local | Commit/status use the trusted local mount store for base; E2E verifies marker base tampering cannot bypass stale-base. |
| 20 | Governance mutation can succeed while event write fails. | P0 | confirmed | open | Make event write part of atomic mutation or add recoverable outbox. |
| 21 | Failed attach can leave half-bound working copy. | P2 | partial | fixed-local | Keep rollback tests; still check failures after event write. |
| 22 | `fs list/status` expose metadata without read grant. | P2 | decision | decision-needed | Decide whether discoverability is allowed; otherwise filter by grant. |
| 23 | `fs attach --json` can leak source credentials. | P0 | confirmed | fixed-local | Use sanitized source result or remove source options from attach JSON. |
| 24 | Grant/revoke target agent id is not validated. | P2 | confirmed | fixed-local | Keep validation tests. |
| 25 | Grant downgrade leaves old capability active. | P0 | confirmed | fixed-local | Keep replacement semantics and tests. |
| 26 | Accepted commit does not record authorizing grant. | P1 | confirmed | fixed-local | Commit records and `commit.accepted` events include `authorized_by`; E2E verifies it through `fs events`. |
| 27 | `fs status <local path>` is documented but unsupported. | P2 | confirmed | fixed-local | `fs status` now resolves local root markers before FS lookup. |
| 28 | `fs status` lacks dirty/materialization state. | P2 | partial | fixed-local | Status now reports materialization, dirty count, stale state, warnings, and next actions. |
| 29 | Successful commit can be reported as failed after marker write failure. | P1 | confirmed | fixed-local | Marker/store update happens after materialization as a local finalization step; marker failure returns a warning while the accepted/materialized commit succeeds. |
| 30 | Directory delete can race child deletes. | P2 | confirmed | open | Order delete plans deepest-first or coalesce subtree deletes. |
| 31 | `fs.attached` leaks local absolute path. | P2 | confirmed | fixed-local | Keep event payload free of local paths. |
| 32 | `commit.materialized` lacks paths/state details. | P2 | confirmed | open | Include path summary and state in event data. |
| 33 | Reattach new empty root can delete remote due to old sync state. | P0 | confirmed | fixed-local | Keep clear-sync-state-before-attach behavior. |
| 34 | Symlinks can expose files outside working root. | P0 | confirmed | fixed-local | Reject symlinks or treat them as unsupported local entry types. |
| 35 | `.section/**` filtering is broader than contract. | P2 | decision | decision-needed | Either reserve all `.section/**` or narrow filters to `.section/agentfs/**` plus local marker. |
| 36 | Attach ignores sync conflicts. | P0 | confirmed | fixed-local | Keep conflict rejection and rollback. |
| 37 | Same local root can be stolen by another source. | P1 | confirmed | fixed-local | Keep cross-source collision rejection; add canonical path check separately. |
| 38 | `fs create` overwrites existing store-owned source. | P1 | confirmed | fixed-local | Keep source-exists guard. |
| 39 | Local root is not canonicalized. | P1 | confirmed | open | Canonicalize existing roots; handle not-yet-created roots carefully. |
| 40 | Bad metadata in unrelated source blocks healthy FS lookup. | P1 | confirmed | open | Only parse candidate source when ref points to source; otherwise collect per-source errors. |
| 41 | Manager can self-grant writer. | P0 | confirmed | fixed-local | Keep self-escalation guard. |
| 42 | Backing root can be attached as working root for `fs` provider. | P0 | confirmed | fixed-local | Reject local roots overlapping provider backing root for local fs provider. |
| 43 | FS ref lookup mishandles local aliases and name collisions. | P1 | confirmed | fixed-local | Lookup now prefers exact fs id, then source name, then FS name; duplicate source/name matches return `ambiguous_fs_ref`. |
| 44 | Commit commands ignore marker `local_root`. | P0 | confirmed | fixed-local | Use marker root or fail on marker/store mismatch. |
| 45 | File/dir type replacement is accepted then fails materialization. | P1 | confirmed | fixed-local | Preflight rejects same-path file/dir type conflicts with `path_type_conflict` before writing accepted commit metadata. |
| 46 | AgentFS tests do not cover the test plan. | P1 | partial | partial | CLI E2E tests now cover the implemented product path; continue converting product-complete gaps into tests as features land. |
| 47 | Overlapping local roots can leak child markers. | P1 | confirmed | open | Reject overlapping roots after canonicalization. |
| 48 | Low-level source commands can break AgentFS markers. | P1 | confirmed | fixed-local | Low-level source/path commands reject AgentFS-backed sources by default. |
| 49 | AgentFS event replay/resume has no control-plane entry. | P0 | covered | fixed-local | `section fs events` supports replay and `--after`; `watch --agentfs` streams the same event records. |
| 50 | `fs status` swallows corrupt local marker. | P2 | confirmed | open | Report marker error in status output. |
| 51 | Event files are not immutable. | P1 | partial | partial | Existing event paths are rejected; still need create-if-absent semantics where backend supports it. |
| 52 | Grant `capabilities` field is not used for enforcement. | P2 | confirmed | fixed-local | Keep enforcement based on stored capabilities; schema validation still needed under #10. |

## Recommended Next Work

### Commit Current Hardening Batch

The current working tree contains a focused hardening batch. Commit it before
starting larger design work.

Recommended commit scope:

- issue triage record,
- non-empty backing source rejection,
- attach/backing root overlap rejection,
- sanitized attach output,
- marker/store local-root mismatch rejection,
- symlink rejection in commit path collection,
- best-effort event immutability check.

### Next Design Pass

Before more code, settle these contract decisions:

1. Is multi-root per FS required inside one local store for MVP?
2. Should `fs list/status` reveal FS metadata without read grants?
3. Is all `.section/**` reserved, or only `.section/agentfs/**` plus local marker?

### Next Implementation Batch

After the decisions above, prioritize:

1. trusted mount base and commit dirty detection (#5, #19, #44),
2. safe FS initialization and recovery (#6, #15, #20),
3. schema validation and backend-level event immutability (#10, #51),
4. local root canonicalization and overlap (#39, #47),
5. bad metadata isolation (#40).

## Verification Record

The current working tree fixes were last checked with:

```bash
cargo fmt --check
cargo test -p section-provider -p sectiond --lib
cargo test -p section-cli --test agentfs_control_plane
cargo test -p section-cli --test agentfs_e2e
cargo test -p section-cli --tests
```
