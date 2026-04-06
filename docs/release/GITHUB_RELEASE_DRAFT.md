# Section `v0.1.0-alpha.1`

## Title

`Section v0.1.0-alpha.1: source/path sync for humans and agents`

## Release Body Draft

Section is a cross-platform source/path sync collaboration layer built for workflows where humans and agents need to work in the same local tree.

This first public alpha establishes the current mainline:

- bind a source to a local directory
- sync regular files and directories into that local tree
- work in normal local paths
- observe sync state through an explicit control plane
- detect stale-overwrite conflicts instead of silently letting timing win

## What ships in this alpha

### Source/path sync core

- persisted source registry
- source to local-root binding
- `.section/root.json` local discovery markers
- source/path sync state persistence
- source/path event persistence
- bidirectional source sync
- explicit stale-overwrite conflict detection

### Control plane

- `section source bind`
- `section source unbind`
- `section source sync`
- `section watch`
- `section path inspect`
- `section path compare`
- `section path resolve --strategy use-local|use-remote`

### User-facing docs

- Quick Start
- User Manual
- Promo / launch copy package

### Install surfaces for this release

- GitHub Release binary archives
- Homebrew tap package
- npm install surface for `npm install -g` and `npx`

## Quick example

```bash
section source add demo --provider fs --opt root=/srv/demo-source
section source bind demo ~/section-demo
section source sync demo
section --json watch ~/section-demo
section --json path compare ~/section-demo/docs/readme.txt
section --json path resolve ~/section-demo/docs/readme.txt --strategy use-local
```

## Who this alpha is for

- developers who want one local tree backed by remote storage
- AI-agent workflows that need stable local paths
- teams who want sync state to stay explicit instead of hidden

## Current non-goals

This alpha does **not** promise:

- full POSIX-mount semantics as the primary product story
- identical cross-platform execution semantics
- strict preservation of every POSIX metadata field
- support for every filesystem object type

## Read next

- [Quick Start](../QUICKSTART.md)
- [User Manual](../USER_MANUAL.md)
- [Product Model](../PRODUCT.md)
- [Sync Model](../SYNC_MODEL.md)

## Visual Assets

- [Launch Square Card](../assets/section-launch-square.png)
- [Social Card](../assets/section-social-card.png)
- [Workflow Card](../assets/section-workflow-card.png)

## Notes

For the first public release, keep the wording honest:

- this is a real usable alpha
- not a marketing-only placeholder
- not a “full mount layer for every platform” claim
