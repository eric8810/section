# Section Active Plan

## Current Route

The active route is:

> `source/path + sync state + local-root binding + event-driven control plane`

## Current Baseline

The current baseline is:

- source/path sync contract
- `sectiond` as the source/path sync core
- source to local-root binding
- `.section/root.json` local discovery
- source/path sync state persistence
- source/path event stream
- bidirectional source sync
- stale-overwrite conflict detection
- explicit `use-local` / `use-remote`
- local-path-aware control plane:
  - `source sync`
  - `watch`
  - `path inspect`
  - `path compare`
  - `path resolve`

## Immediate Direction

Current active docs should be read as present tense product/architecture, not as a future pivot plan.

If development continues from here, it should extend the implemented `source/path` sync model rather than reintroduce parallel abstractions or archived routes.
