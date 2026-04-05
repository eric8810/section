# Sectiond Boundary

## Why this doc exists

`docs/ARCHITECTURE.md` explains the product direction. This document makes that direction operational by defining the first concrete contract for `sectiond`.

The short version:

- `FS` is the primary working surface
- `CLI / API` are the control plane
- `sectiond` is the future single local runtime center

This repo is still in transition, so the goal of this document is not to pretend the daemon already exists. The goal is to define what will move into it and what must stay outside it.

## Product invariant

Section is only on the right path if these things stay true:

1. Humans, agents, shell tools, and editors can work on the same mounted namespace.
2. Platform differences live in mount adapters, not in the product mental model.
3. CLI/API may help manage the system, but they do not replace the shared mounted workspace as the main collaboration surface.

## sectiond ownership

The long-lived local runtime should own the shared semantics that must not diverge between clients.

### sectiond owns

- source registry
- operator lifecycle
- routing
- metadata/content cache
- refresh/invalidation
- permissions/conflict semantics
- health and diagnostics
- runtime sessions / lifecycle state

### sectiond does not own

- parsing human CLI flags
- rendering terminal output
- direct platform mount syscalls
- GUI-specific presentation

Those remain client or adapter responsibilities.

## Surface split

### Control plane

These are client-facing management actions:

- source add/remove/list
- status / diagnostics
- refresh / repair / health checks
- config/bootstrap/preflight
- explicit fallback flows when mount is unavailable

Today these mainly live in `section-cli`. Over time, `section-cli` should become a client of `sectiond`, not a parallel runtime center.

### Data plane

These are the semantics that mounted workspace users depend on:

- path traversal
- list/read/write/delete/rename
- cache-backed visibility
- permission checks
- refresh visibility
- shell/script/editor access against the mounted tree

Linux and macOS adapters should expose these semantics through the same `sectiond`-owned state machine.

## Runtime boundary

The first practical boundary is:

1. load local config
2. materialize a merged source registry
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

This is intentional and explicit. The next issue (`#14`) is where these semantics should consolidate into a single authoritative local state machine.

## Immediate follow-up mapping

- `#13`
  - define boundary, contract, ownership, lifecycle
  - create the first concrete `sectiond` crate/snapshot boundary
- `#14`
  - move shared semantics into `sectiond`
- `#15`
  - move CLI toward control-plane client behavior
- `#16`
  - route Linux mount adapter through `sectiond`
- `#17`
  - formalize execute/scripting on the mounted workspace
- `#18` + `#11`
  - productize and validate the macOS adapter path
