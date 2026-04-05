# Sync Workspace Contract

## Purpose

This document defines the new product mainline for Section after the pivot away from mount-first delivery.

The mainline question is now:

> What does Section guarantee when it gives humans and agents a local workspace that is backed by remote sources?

## Core Promise

Section guarantees a **truthful local workspace**, not a cross-platform POSIX illusion.

That means:

- files and directories can be materialized locally
- local and remote changes can be reconciled
- sync, pending, conflict, and readiness states are visible
- humans and agents can work against the same local directory

It does **not** mean:

- all platforms have identical host-native execution behavior
- full POSIX metadata round-trips across all backends and OSes
- every object type is preserved in MVP

## Workspace Object Model

MVP-supported object classes:

- regular files
- directories

Deferred object classes:

- symlinks
- hardlinks
- device files
- sockets
- FIFOs
- complex xattr-heavy objects

## State Model

### Object Readiness

Each object should surface at least:

- `materialized`
- `not_materialized`
- `syncing`
- `dirty_local`
- `dirty_remote`
- `conflict`
- `error`

### Workspace Health

Each workspace should surface at least:

- `healthy`
- `syncing`
- `offline`
- `degraded`
- `conflict_present`

## Metadata Policy

MVP should preserve:

- path
- type
- size
- modified time where practical
- content hash/version where practical

MVP should not promise strict preservation of:

- uid/gid
- owner/group
- ACL
- execute bit across all platforms
- xattr

If some of these are present on a specific platform/backend combination, they are best-effort enhancements, not the primary contract.

## Materialization Model

Section needs explicit states for:

- on-demand materialization
- pinned local content
- evictable local content
- stale local content awaiting refresh

The product should prefer explicitness over pretending all visible files are equally ready.

## Conflict Model

Conflict is not an edge case; it is a first-class workspace state.

Section should define:

- what counts as concurrent local/remote divergence
- how conflict surfaces to user and agent
- what automatic merges are allowed, if any
- what repair actions exist

## Execution Boundary

Section does not promise that every file visible in the workspace is immediately safe to execute on every host.

Section should promise only:

- readiness/materialization visibility
- clear local vs remote state
- truthful workspace state

Execution must be defined separately by runtime policy, for example:

- explicit interpreters
- containers
- WSL
- remote POSIX runners

## User-Facing Truth

The product must be honest about:

- which files are local
- which files are only placeholders or not yet materialized
- which files are dirty
- which files are conflicted
- which objects fall outside MVP fidelity

## Future Extensions

These may be layered later on top of the same contract:

- FUSE-based advanced mode
- macOS File Provider
- Windows Cloud Files API
- SMB export

But they should consume this contract rather than define it.
