# Source/Path Sync Contract

## Purpose

This document defines the active Section sync contract.

## Core Promise

Section guarantees a truthful source/path sync model.

That means:

- sources remain the configured objects
- paths remain the content objects
- files and directories sync into a local directory
- local and remote changes can be reconciled
- state remains simple and truthful

## Source Model

Each source defines:

- backend/provider identity
- remote root
- optional local root
- source health
- source sync policy

## Path Model

MVP object classes:

- regular files
- directories

Deferred object classes:

- symlinks
- hardlinks
- device files
- sockets
- FIFOs
- xattr-heavy objects

## State Model

### Public state

Each source and path exposes:

- `ready`
- `syncing`
- `conflict`
- `error`

### Detail fields

Detail output may include:

- `local_present`
- `dirty_local`
- `dirty_remote`
- `pinned`
- `stale`
- health reason
- error reason

## Metadata Policy

MVP preserves:

- path
- type
- size
- modified time where practical
- content hash/version where practical

MVP does not promise strict preservation for:

- uid/gid
- owner/group
- ACL
- execute bit across platforms
- xattr

## Local Presence

Section tracks:

- source local-root binding
- local file presence
- stale local content

These remain detail fields, not primary state names.

## Conflict Model

Conflict means stale-overwrite protection.

In MVP:

- a local upload based on stale remote state enters `conflict`
- sync for that path pauses
- the local file is preserved
- the current remote version is not overwritten automatically

Resolution actions are:

- `use-local`
- `use-remote`

## Control Plane Surface

The local file tree is the data plane.

The control plane exposes:

- `watch`
  - source/path state-change events
- `path inspect`
  - public state
  - detail fields
  - `base_remote_version`
  - `current_remote_version`
- `path compare`
  - local vs current remote truth
- `path resolve --strategy use-local|use-remote`
  - explicit conflict resolution

## Local Discovery

Each bound local root contains:

- `.section/root.json`

It exists only for discovery and minimally identifies:

- `source_id`
- `local_root`
- control-plane endpoint

Preferred flow:

1. subscribe once with `watch`
2. let the client discover `.section/root.json` internally
3. react to events
4. call `inspect` / `compare` only when needed
5. call `resolve` when action is required

Common control-plane entry points should accept local paths directly.

## Execution Boundary

Execution is outside the current project scope.

## User-Facing Truth

The product must be honest about:

- which paths are `ready / syncing / conflict / error`
- which sources are `ready / syncing / conflict / error`
- which files are already local when detail is requested
- which files are dirty when diagnostics require it
- which files are conflicted
