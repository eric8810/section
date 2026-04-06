# Section Architecture

## Goal

Section provides a cross-platform source/path sync layer so humans and agents can collaborate on the same local directory tree.

## Core Principles

### 1. Source/path is primary

The product model is:

- sources
- paths
- local-root bindings
- sync state

### 2. sectiond is the authoritative state machine

`sectiond` owns:

- source registry
- local-root bindings
- source/path state
- source/path detail fields
- conflict handling
- event emission
- health and diagnostics

### 3. The local tree is the work surface

Humans, editors, shells, and agents work against local paths.

### 4. The control plane carries sync truth

The local filesystem is the data plane.

The control plane carries:

- watch/event subscription
- inspect
- compare
- resolve
- diagnostics

### 5. Execution is outside the current project scope

The current project defines sync behavior, not cross-platform execution semantics.

## Runtime Model

```text
Humans / Agents / Shell / Editors
               |
       Local Source Trees
               |
           sectiond core
 source registry / sync state / detail state / events
 local change ingest / remote change ingest / conflicts
               |
         Control Plane
      (CLI / GUI / API)
               |
            OpenDAL
     S3 / WebDAV / fs / ...
```

## Responsibilities

### sectiond

`sectiond` owns:

- source registry
- source local-root bindings
- `.section/root.json` discovery metadata
- routing
- source/path public state
- source/path detail state
- event stream
- metadata/content cache
- conflict detection and resolution state
- health and diagnostics

### section-cli

CLI acts as a control-plane client for:

- source management
- source sync
- status / diagnostics
- watch
- inspect / compare / resolve

### Local Source Tree

The local tree supports:

- normal file and directory access
- shared human/agent workflows
- local edits that can be detected and synchronized

## State Model

### Public state

Each source and path exposes:

- `ready`
- `syncing`
- `conflict`
- `error`

### Detail fields

Internal or detailed views may include:

- `local_present`
- `dirty_local`
- `dirty_remote`
- `pinned`
- `stale`
- health reason
- error reason

## Data Flow

### Remote to Local

1. remote source emits or is polled for change
2. `sectiond` updates source/path state
3. local tree is reconciled
4. event stream reports the state change

### Local to Remote

1. user or agent edits a local file
2. local change is observed
3. `sectiond` stages sync work
4. remote write succeeds, or stale remote state is detected
5. if stale, the path enters `conflict`
6. explicit `use-local` or `use-remote` resolves the path

## Current Project Boundary

The current project is about:

- source/path sync
- local-root binding
- state visibility
- event-driven notification
- conflict resolution
- control-plane ergonomics
