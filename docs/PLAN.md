# Section Active Plan

## Active Route

The active route is:

> `source/path + sync state + local-root binding + event-driven control plane`

## Phases

### Phase 1: Source/Path Sync Contract

Deliver:

- source model
- public state model
- detail fields model
- metadata scope
- non-goals

### Phase 2: sectiond Sync Core

Deliver:

- source registry
- local-root binding ownership
- sync state ownership
- event-stream ownership
- sync scheduler

### Phase 3: Local-Root Binding and Detail State

Deliver:

- local-root binding
- `.section/root.json`
- local path state persistence
- detail inspection

### Phase 4: Bidirectional Sync and Conflict Resolution

Deliver:

- local change ingest
- remote change ingest
- bidirectional reconciliation
- stale-overwrite detection
- explicit `use-local` / `use-remote`

### Phase 5: Control Plane

Deliver:

- source sync bind/status
- local-path-aware watch / event subscribe
- local-path-aware path inspect
- local-path-aware path compare
- local-path-aware path resolve
- sync / pull / pin / repair
- conflict inspection

## Active Issue Map

| Phase | Theme | GitHub issue |
|------|------|--------------|
| Phase 1 | source/path sync contract and non-goals | `#19` |
| Phase 2 | sectiond as the source/path sync core | `#22` |
| Phase 3 | source local-root binding and path detail state | `#21` |
| Phase 4 | bidirectional sync and conflict resolution | `#20` |
| Phase 5 | source/path control plane | `#24` |

## Execution Order

1. `#19`
2. `#22`
3. `#21`
4. `#20`
5. `#24`
