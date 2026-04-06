# Sectiond Boundary

## Purpose

This document defines the active ownership boundary for `sectiond`.

## Invariants

These stay true:

1. `source/path` remains the primary model
2. sync and conflict live on sources and paths
3. public state stays at `ready / syncing / conflict / error`
4. control-plane clients consume one authoritative `sectiond` state model

## sectiond Owns

- source registry
- source local-root bindings
- `.section/root.json` marker metadata
- routing
- source/path public state
- source/path detail state
- source/path event stream
- metadata/content cache
- refresh / invalidation
- conflict semantics
- health and diagnostics
- runtime lifecycle state

## sectiond Does Not Own

- human CLI flag parsing
- terminal rendering
- GUI presentation

## Control Plane Surface

Control-plane clients should expose:

- source add / remove / list
- source bind-local-root / unbind-local-root
- source sync / source status
- watch / event subscribe
- path pull / pin / inspect / compare / resolve / repair
- status / diagnostics
- refresh / repair / health checks
- config / bootstrap / preflight

Common control-plane entry points should accept local paths directly and perform `.section/root.json` discovery internally.

## Data Plane

The data plane is the local bound tree used for:

- traversal
- list / read / write / delete / rename
- shared human/agent editing

## Runtime Boundary

The runtime boundary is:

1. load local config
2. load source registry
3. build routing and runtime state
4. expose that state to control-plane clients
5. emit state-change events

## Active Mapping

- `#19`
  - source/path sync contract and non-goals
- `#22`
  - sectiond as the source/path sync core
- `#21`
  - local-root binding and path detail state
- `#20`
  - bidirectional sync and conflict resolution
- `#24`
  - source/path control plane
