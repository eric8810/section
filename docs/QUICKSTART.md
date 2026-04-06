# Section Quick Start

## Goal

In 5 minutes, you should be able to:

- add a source
- bind it to a local directory
- sync remote content into that directory
- watch sync events
- inspect / compare / resolve a path by local path

## Example Setup

This example uses a local filesystem source so the flow is easy to reproduce.

- remote source root: `/srv/section-demo-source`
- local bound root: `~/section-demo`

## 1. Add a source

```bash
section source add demo --provider fs --opt root=/srv/section-demo-source
```

Expected shape:

```text
Source 'demo' added (provider: fs).
```

## 2. Bind the source to a local directory

```bash
section source bind demo ~/section-demo
```

Expected shape:

```text
Source 'demo' bound to local root /Users/you/section-demo.
```

Section will place a lightweight discovery marker at:

```text
~/section-demo/.section/root.json
```

That marker is only for local discovery. Sync truth still comes from the control plane.

## 3. Run sync

```bash
section source sync demo
```

Expected shape:

```text
Source 'demo' synced. local_root=/Users/you/section-demo, pulled=2, pushed=0, conflicts=0, events=2
```

At this point, files and directories from the remote source should exist under the bound local root.

## 4. Inspect a path by local path

```bash
section --json path inspect ~/section-demo/docs/readme.txt
```

Example output shape:

```json
{
  "source_id": "demo",
  "local_root": "/Users/you/section-demo",
  "local_path": "/Users/you/section-demo/docs/readme.txt",
  "source_path": "docs/readme.txt",
  "state": "ready",
  "detail": {
    "local_present": true,
    "dirty_local": false,
    "dirty_remote": false,
    "pinned": false,
    "stale": false
  },
  "base_remote_version": "<hash>",
  "current_remote_version": "<hash>"
}
```

## 5. Compare local truth against remote truth

```bash
section --json path compare ~/section-demo/docs/readme.txt
```

Example output shape:

```json
{
  "source_id": "demo",
  "source_path": "docs/readme.txt",
  "entry_kind": "file",
  "state": "ready",
  "local_present": true,
  "remote_present": true,
  "local_version": "<hash>",
  "base_remote_version": "<hash>",
  "current_remote_version": "<hash>",
  "local_matches_base": true,
  "local_matches_current_remote": true,
  "stale": false
}
```

Use `path compare` when you need to know whether the local file is still based on the current remote version.

## 6. Watch events instead of polling every file

```bash
section --json watch ~/section-demo
```

Example output shape:

```json
{"id":1,"source_name":"demo","path":"docs","kind":"synced_from_remote","state":"ready","created_at_ms":1775451218105}
{"id":2,"source_name":"demo","path":"docs/readme.txt","kind":"synced_from_remote","state":"ready","created_at_ms":1775451218106}
```

Recommended agent loop:

1. subscribe once with `watch`
2. react only to paths named by events
3. call `path inspect` or `path compare` on demand
4. call `path resolve` only when action is required

## 7. Resolve a conflict explicitly

If both local and remote changed since the last common remote base, Section enters `conflict` and refuses blind overwrite.

Check the conflicted path:

```bash
section --json path compare ~/section-demo/docs/readme.txt
```

Conflict output shape:

```json
{
  "state": "conflict",
  "local_matches_base": false,
  "local_matches_current_remote": false,
  "stale": true
}
```

Then choose a resolution:

```bash
section --json path resolve ~/section-demo/docs/readme.txt --strategy use-local
```

Or:

```bash
section --json path resolve ~/section-demo/docs/readme.txt --strategy use-remote
```

Resolution output shape:

```json
{
  "source_id": "demo",
  "source_path": "docs/readme.txt",
  "strategy": "use-local",
  "state": "ready",
  "base_remote_version": "<hash>",
  "current_remote_version": "<hash>"
}
```

## Mental Model

- work in the local tree for normal editing
- use the control plane for sync truth
- use `watch` for notification
- use `inspect` / `compare` / `resolve` for decisions

## Next Read

- [USER_MANUAL.md](USER_MANUAL.md)
- [PRODUCT.md](PRODUCT.md)
- [SYNC_MODEL.md](SYNC_MODEL.md)
