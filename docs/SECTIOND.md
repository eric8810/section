# Sectiond Boundary

## Why this doc exists

`docs/ARCHITECTURE.md` explains the product direction. This document makes that direction operational by defining the first concrete contract for `sectiond`.

The short version:

- `source/path` is the primary working surface
- `CLI / API` are the control plane
- `sectiond` is the future single local runtime center

This repo is still in transition, so the goal of this document is not to pretend the daemon already exists. The goal is to define what will move into it and what must stay outside it.

## Product invariant

Section is only on the right path if these things stay true:

1. `source/path` stays the only primary product mental model.
2. Sync and conflict live on sources and paths, not in a separate top-level abstraction.
3. Public state stays simple: `ready / syncing / conflict / error`.
4. CLI/API must consume the same `sectiond`-owned source/path state model.

## sectiond ownership

The long-lived local runtime should own the shared semantics that must not diverge between clients.

### sectiond owns

- source registry
- source local-root bindings
- operator lifecycle
- routing
- source/path public state
- source/path detail state
- metadata/content cache
- refresh/invalidation
- permissions/conflict semantics
- health and diagnostics
- runtime sessions / lifecycle state

### sectiond does not own

- parsing human CLI flags
- rendering terminal output
- GUI-specific presentation

Those remain client responsibilities.

## Surface split

### Control plane

These are client-facing management actions:

- source add/remove/list
- source bind-local-root / unbind-local-root
- source sync / source status
- path pull / pin / inspect / repair
- status / diagnostics
- refresh / repair / health checks
- config/bootstrap/preflight

Today these still surface through `section-cli`, but the route-map direction is explicit: those commands should enter through `sectiond`, not keep accumulating parallel runtime logic in the CLI process.

### Data plane

These are the semantics that users depend on:

- path traversal
- list/read/write/delete/rename
- cache-backed visibility
- source/path state visibility
- refresh visibility
- shell/script/editor access against the local tree when configured


## Runtime boundary

The first practical boundary is:

1. load local config
2. build a merged source registry
3. build routing/runtime state once
4. expose it to control-plane and data-plane clients

That is why the repo now contains a `crates/sectiond` workspace member. It is not the final daemon yet. It is the first concrete runtime boundary for:

- merged source loading
- runtime snapshotting
- explicit contract ownership

## Transitional truth

Right now the runtime boundary still has transitional behavior:

- source definitions can still come from both config and `ProviderStore`
- when the same source exists in both places, config-file entries still win
- `sectiond` is a skeleton crate, not the final daemon process

This is intentional and explicit. The next pivot issue (`#22`) is where these semantics should consolidate into a single authoritative local state machine.

## Immediate follow-up mapping

- `#19`
  - define the source/path sync contract and non-goals
- `#22`
  - move shared source/path sync semantics into `sectiond`
- `#21`
  - add source local-root binding and path detail state
- `#20`
  - add bidirectional sync and conflict resolution
- `#24`
  - move CLI toward source/path control-plane behavior
