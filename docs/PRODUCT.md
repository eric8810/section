# Section Product Model

## One-Line Definition

Section is a cross-platform `source/path` sync collaboration layer.

## Product Promise

Section promises:

- `source/path` stays the primary model
- a source can bind to a local directory
- regular files and directories sync into that local tree
- humans and agents work against the same local paths
- state is visible and controllable
- public state stays at `ready / syncing / conflict / error`

Section does not promise:

- identical host-native execution semantics across platforms
- strict round-trip preservation for every POSIX metadata field
- support for every filesystem object type in MVP

## Access Surfaces

| Surface | Role |
|--------|------|
| Local file tree | daily read/write surface for humans, editors, shells, and agents |
| Control plane | source/path status, compare, resolve, sync, and diagnostics |
| Runtime layer | execution environment outside the current project scope |

## Source/Path Contract

### Supported objects

- regular files
- directories

### Public state

Each source and path exposes only:

- `ready`
- `syncing`
- `conflict`
- `error`

### Detail fields

The following remain detail fields rather than primary state:

- `local_present`
- `dirty_local`
- `dirty_remote`
- `pinned`
- `stale`
- health reason
- error reason

### Metadata scope

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
- execute bit across all platforms
- xattr
- symlink / hardlink
- device files / sockets / FIFOs

## Conflict Model

MVP conflict is stale-overwrite protection.

`conflict` means:

- a local upload is based on stale remote state
- Section refuses a blind overwrite

When conflict happens:

- the path enters `conflict`
- sync for that path pauses
- the local file is preserved
- the current remote version is not overwritten automatically

Resolution actions are:

- `use-local`
- `use-remote`

If a user merges manually in an editor, the final action is still `use-local`.

## Control Plane

The control plane exists because the local file itself cannot reliably express sync truth.

Minimum control-plane surface:

- `watch`
  - event stream for source/path state changes
- `path inspect`
  - public state
  - detail fields
  - `base_remote_version`
  - `current_remote_version`
- `path compare`
  - whether local is based on current remote
  - local/remote compare references
- `path resolve --strategy use-local|use-remote`
  - explicit conflict resolution

## Local Discovery

Each bound local root should contain:

- `.section/root.json`

It exists only for discovery and should minimally identify:

- `source_id`
- `local_root`
- `sectiond` control-plane endpoint

The preferred agent flow is:

1. subscribe once with `watch` from a local path or bound root
2. let the client discover `.section/root.json` internally
3. react to state-change events
4. call `inspect` / `compare` only when needed
5. call `resolve` when action is required

Common control-plane entry points should accept local paths directly:

- `section --json watch ./`
- `section --json path inspect ./some/local/file`
- `section --json path compare ./some/local/file`
- `section --json path resolve ./some/local/file --strategy use-local`

## MVP

MVP should deliver:

1. connect one or more sources
2. bind a source to a local directory
3. sync regular files and directories into that local tree
4. converge local and remote changes
5. expose `ready / syncing / conflict / error`
6. expose control-plane status / compare / resolve / watch surfaces
