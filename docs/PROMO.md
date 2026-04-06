# Section Promo Package

## Direct Positioning

### One-line definition

Section turns remote sources into a shared local working tree for humans and agents.

### Short version

Bind a source. Sync it into a local directory. Work there normally. Use the control plane only for sync truth.

### Who it is for

- developers working across multiple storage backends
- AI-agent workflows that need stable local paths
- teams that want one local tree instead of ad hoc backend-specific tooling

## Core Story

Most tools make you choose between:

- backend-specific APIs
- mount-first complexity
- or sync tools that hide state until something goes wrong

Section’s story is simpler:

- keep `source/path` as the model
- keep the local tree as the work surface
- keep sync truth explicit
- keep conflict resolution deliberate

## Three Core Messages

1. **One local tree**
   - humans and agents work against the same local paths
2. **Truthful sync state**
   - `ready / syncing / conflict / error` is explicit instead of hidden
3. **Agent-ready control plane**
   - `watch`, `inspect`, `compare`, and `resolve` are built for automation

## Landing Page Draft

### Hero

Headline:

> One local tree for humans and agents.

Subheadline:

> Section turns remote sources into a truthful local work surface with explicit sync state, event-driven watch, and deliberate conflict resolution.

Primary CTA:

> Read the Quick Start

Secondary CTA:

> See the sync model

### Supporting bullets

- Bind remote sources to local directories
- Keep daily work in normal local paths
- Subscribe once to sync events instead of polling
- Compare and resolve by local path when state matters

### Product blurb

Section is a cross-platform `source/path` sync collaboration layer built on Apache OpenDAL. It is designed for workflows where humans, shells, editors, and agents need to operate on the same local tree without giving up truthful sync visibility.

## Demo Story

Use this order for a 30 to 60 second demo:

1. add a source
2. bind it to a local root
3. run `source sync`
4. open the local tree in a normal editor or shell
5. start `watch`
6. trigger a local/remote divergence
7. show `path compare`
8. resolve with `use-local` or `use-remote`

## Launch Copy

### Short post

> Section is a source/path sync layer for humans and agents.\n> Bind a remote source to a local directory, work in normal paths, and keep sync truth explicit with `watch`, `inspect`, `compare`, and `resolve`.

### Slightly longer version

> We built Section because “just mount it” and “just sync it” both break down once humans and agents share the same working tree. Section keeps the model simple: sources and paths stay explicit, the local tree stays usable, and sync truth stays visible.

## Visual Direction

Use the following design language:

- warm paper background instead of flat white
- black typography with one strong signal color
- editorial layout, not generic SaaS gradients
- terminal and architecture snippets used as evidence, not decoration
- short, assertive copy

## Included Visual Assets

- [assets/section-launch-square.svg](assets/section-launch-square.svg)
- [assets/section-launch-square.png](assets/section-launch-square.png)
- [assets/section-social-card.svg](assets/section-social-card.svg)
- [assets/section-social-card.png](assets/section-social-card.png)
- [assets/section-workflow-card.svg](assets/section-workflow-card.svg)
- [assets/section-workflow-card.png](assets/section-workflow-card.png)

## Recommended Asset Uses

### `section-launch-square.svg`

Use for:

- DM / chat preview image
- square social post
- launch announcement card

### `section-social-card.svg`

Use for:

- repo social preview
- launch post artwork
- docs banner

### `section-workflow-card.svg`

Use for:

- documentation hero image
- launch thread step explainer
- quick architecture/story slide
