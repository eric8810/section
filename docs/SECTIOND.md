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
- source sync
- watch / event subscribe
- path inspect / compare / resolve
- status / diagnostics
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

## AgentFS Control Service

`sectiond serve` runs the HTTP Section Control Service used by AgentFS
cross-machine sharing.

```bash
sectiond --config server.toml serve --addr 127.0.0.1:7373
```

Agent clients point at it with:

```toml
[control_service]
endpoint = "http://127.0.0.1:7373"
```

In this mode, client configs do not contain SourceProfile or backing-source
credentials. The service owns agent identity, installation identity, grants,
shares, SourceProfile selection, credential issuance, and service-side AgentFS
events.
