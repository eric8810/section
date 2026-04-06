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

- what counts as a stale overwrite attempt
- how conflict surfaces to user and agent
- what repair actions exist

### MVP Conflict Policy

MVP should use explicit resolution, not auto-merge or version branches.

In MVP, `conflict` means one thing:

- local upload is based on an older remote version, and Section refuses a blind overwrite

When a conflict is detected:

- the path state becomes `conflict`
- automatic sync for that path stops
- the local current file is preserved
- the remote current version is not overwritten automatically

The allowed resolution actions are:

- `use-local`
- `use-remote`

If a user wants to merge manually in an editor, that still ends as `use-local` after the merged local file is ready.

After an explicit resolution, the path can return to normal sync.

## Observation and Resolution Surface

The local file tree is the data plane. It is not the full sync-control surface.

A normal editor or shell only sees:

- local file content
- normal filesystem metadata

It does not reliably know:

- current sync state
- current remote version
- whether the local file is based on stale remote state
- how to choose `use-local` or `use-remote`

That information must come from the control plane.

At minimum, the control plane should expose:

- `watch`
  - source/path state-change events
  - a long-lived notification surface so agents do not poll every file
- `path inspect`
  - public state
  - detail fields
  - `base_remote_version`
  - `current_remote_version`
- `path compare`
  - whether local is based on current remote
  - local/remote compare references or snapshot information
- `path resolve --strategy use-local|use-remote`
  - explicit stale-overwrite resolution

So the contract is:

- ordinary apps work against local files
- Section-aware clients query and control sync state through the control plane

For normal agent ergonomics, these control-plane entry points should accept a local path inside a bound root, not force the caller to manually reconstruct `source/path`.

For normal agent notification semantics, the preferred model is subscribe-then-inspect, not poll-then-guess.

## Local Discovery Entry

If a bound local root has no local discovery entry, agents cannot reliably know they are inside a Section-managed tree.

So each bound local root should contain one lightweight marker:

- `.section/root.json`

Its purpose is only discovery, not full sync-state storage.

At minimum it should identify:

- `source_id`
- `local_root`
- control-plane endpoint for `sectiond`

The discovery algorithm is:

1. start from the current path
2. walk up parent directories
3. stop when `.section/root.json` is found
4. treat that directory as the bound local root
5. use the marker to query the control plane for sync truth

This keeps the split clean:

- local files remain normal files
- discovery is local and cheap
- sync truth still comes from the control plane

The preferred call flow is:

1. agent subscribes once via `watch` on a local path or bound root
2. the client discovers `.section/root.json` internally
3. an event indicates which source/path changed state
4. agent calls `path inspect` / `path compare` only when needed
5. agent calls `path resolve` if explicit action is required

## Execution Boundary

Execution is outside the current project scope.

The current project promises only:

- public state visibility
- local presence detail when needed
- clear local vs remote state
- truthful source/path state

## User-Facing Truth

The product must be honest about:

- which paths are `ready / syncing / conflict / error`
- which files are already local when users need that detail
- which files are dirty when diagnostics require it
- which files are conflicted
- which sources are `ready / syncing / conflict / error`
- which objects fall outside MVP fidelity
