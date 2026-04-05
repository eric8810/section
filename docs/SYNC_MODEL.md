# Source/Path Sync Contract

## Purpose

This document defines the new product mainline for Section after the pivot away from mount-first delivery.

The mainline question is now:

> What does Section guarantee when it exposes sources and paths with truthful sync and conflict state?

## Core Promise

Section guarantees a **truthful source/path model**, not a cross-platform POSIX illusion.

That means:

- sources remain the primary configured objects
- paths remain the primary content objects
- files and directories can be synced into a local directory
- local and remote changes can be reconciled
- user-facing state stays simple and truthful

It does **not** mean:

- all platforms have identical host-native execution behavior
- full POSIX metadata round-trips across all backends and OSes
- every object type is preserved in MVP

## Source Model

Each source should define:

- backend/provider identity
- remote root
- optional local root
- source health
- source sync mode / policy

## Path Object Model

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

### Public Path State

Each path should expose only:

- `ready`
- `syncing`
- `conflict`
- `error`

### Public Source State

Each source should expose only:

- `ready`
- `syncing`
- `conflict`
- `error`

### Detail Fields

The following should stay out of the main user-facing state model and only appear in details / diagnostics / machine-readable output:

- `local_present`
- `dirty_local`
- `dirty_remote`
- `pinned`
- `stale`
- health reason
- error reason

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

## Local Presence Model

Section still needs internal detail for:

- source local-root binding
- local file presence
- pinned local content
- evictable local content
- stale local content awaiting refresh

But these should not become the primary user-facing state names.

## Conflict Model

Conflict is not an edge case; it is a first-class source/path state.

Section should define:

- what counts as concurrent local/remote divergence
- how conflict surfaces to user and agent
- what automatic merges are allowed, if any
- what repair actions exist

## Execution Boundary

Section does not promise that every file visible under a source path is immediately safe to execute on every host.

Section should promise only:

- public state visibility
- local presence detail when needed
- clear local vs remote state
- truthful source/path state

Execution must be defined separately by runtime policy, for example:

- explicit interpreters
- containers
- WSL
- remote POSIX runners

## User-Facing Truth

The product must be honest about:

- which paths are `ready / syncing / conflict / error`
- which files are already local when users need that detail
- which files are dirty when diagnostics require it
- which files are conflicted
- which sources are `ready / syncing / conflict / error`
- which objects fall outside MVP fidelity
