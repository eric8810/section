# Section User Manual

## What Section Is

Section is a cross-platform `source/path` sync collaboration layer.

In practice, that means:

- a remote source is configured once
- that source can bind to a local directory
- humans and agents work against the same local paths
- sync truth is visible through the control plane

Section is not trying to be:

- a full POSIX-preserving mount layer in the current scope
- a cross-platform execution runtime
- a hidden auto-merge system

## Core Mental Model

There are only four concepts that matter day to day:

- `source`
  - a configured backend/root such as `fs`, `s3`, or `webdav`
- `local root`
  - the local directory bound to that source
- `local tree`
  - where humans, editors, shells, and agents do normal file work
- `control plane`
  - where sync truth, version truth, compare, and resolve live

## Public State

Section exposes only four user-facing states:

- `ready`
- `syncing`
- `conflict`
- `error`

Detail fields such as `dirty_local`, `dirty_remote`, `local_present`, `pinned`, and `stale` exist, but they are diagnostics, not the main mental model.

## Command Map

| Goal | Command |
|------|---------|
| Add a source | `section source add <name> --provider ... --opt ...` |
| Bind local root | `section source bind <name> <local_root>` |
| Remove local binding | `section source unbind <name>` |
| Run sync | `section source sync <name>` |
| Watch events | `section --json watch <local_path_or_root>` |
| Inspect sync state | `section --json path inspect <local_path>` |
| Compare local vs remote | `section --json path compare <local_path>` |
| Resolve conflict | `section --json path resolve <local_path> --strategy use-local|use-remote` |

## Standard Workflow

### 1. Create and bind a source

```bash
section source add demo --provider fs --opt root=/srv/section-demo-source
section source bind demo ~/section-demo
```

After binding, Section creates:

```text
~/section-demo/.section/root.json
```

That file is a discovery marker. It lets CLI/API clients recognize that a local path belongs to a Section-bound root.

### 2. Run sync

```bash
section source sync demo
```

This reconciles the source and the bound local tree.

### 3. Work in the local tree

Use normal tools:

- editors
- shell commands
- scripts
- agents

Example:

```bash
cd ~/section-demo
ls
cat docs/readme.txt
```

### 4. Watch for changes

Instead of polling every file, subscribe once:

```bash
section --json watch ~/section-demo
```

Example event:

```json
{"id":3,"source_name":"demo","path":"docs/readme.txt","kind":"conflict_detected","state":"conflict","created_at_ms":1775451230137}
```

### 5. Inspect or compare only when needed

Inspect a path:

```bash
section --json path inspect ~/section-demo/docs/readme.txt
```

Use compare when you need freshness truth:

```bash
section --json path compare ~/section-demo/docs/readme.txt
```

## Conflict Handling

Section uses stale-overwrite protection.

That means:

- if local changed
- and remote also changed after the last known remote base
- Section does not silently let one side win by timing

Instead:

- the path enters `conflict`
- sync pauses for that path
- the local file remains in place
- the current remote version is preserved

### Resolve with local

```bash
section --json path resolve ~/section-demo/docs/readme.txt --strategy use-local
```

Use this when the local file should overwrite the current remote truth.

### Resolve with remote

```bash
section --json path resolve ~/section-demo/docs/readme.txt --strategy use-remote
```

Use this when the current remote file should replace the local one.

## What Agents Should Do

Recommended agent flow:

1. accept a local filesystem path as the entry point
2. subscribe once with `watch`
3. react to named paths from the event stream
4. call `path inspect` only when state/detail is needed
5. call `path compare` when freshness truth matters
6. call `path resolve` only for explicit human or policy decisions

Agents should not:

- infer sync truth from the file contents alone
- repeatedly poll `inspect` on every file
- guess stale/remote state from timestamps without the control plane

## JSON Field Guide

### `path inspect`

Use when you need:

- public state
- detail state
- base/current remote version references

Key fields:

- `state`
- `detail.local_present`
- `detail.dirty_local`
- `detail.dirty_remote`
- `detail.stale`
- `base_remote_version`
- `current_remote_version`

### `path compare`

Use when you need:

- local vs current remote truth
- stale decision support

Key fields:

- `local_version`
- `base_remote_version`
- `current_remote_version`
- `local_matches_base`
- `local_matches_current_remote`
- `stale`

### `watch`

Use when you need:

- event-driven notification
- one subscription instead of per-file polling

Key fields:

- `id`
- `source_name`
- `path`
- `kind`
- `state`
- `created_at_ms`

## What Section Does Not Promise

Current scope does not promise:

- identical host-native execution semantics across platforms
- strict preservation of every POSIX metadata field
- automatic merge for file conflicts
- hidden sync state encoded directly inside ordinary files

## Suggested Reading Order

1. [QUICKSTART.md](QUICKSTART.md)
2. [PRODUCT.md](PRODUCT.md)
3. [SYNC_MODEL.md](SYNC_MODEL.md)
4. [SECTIOND.md](SECTIOND.md)
