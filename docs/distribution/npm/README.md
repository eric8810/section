# Section npm Distribution

## Direct Recommendation

Publish a scoped CLI package:

- npm package: `@eric8810/section`

That gives Section two real install surfaces from the same package:

```bash
npm install -g @eric8810/section
section --help
sectiond inspect
```

```bash
npx --package @eric8810/section section --help
```

## Why this route

- npm is an install surface, not just a JS ecosystem channel
- it gives a low-friction CLI install story for users already living in Node tooling
- it gives an `npx` entry for one-shot use
- it does not force Section to promise a JavaScript SDK

Official references:

- npm scoped public packages: <https://docs.npmjs.com/creating-and-publishing-scoped-public-packages/>
- npm package.json `bin` behavior: <https://docs.npmjs.com/cli/v11/configuring-npm/package-json>

## Package Model

The npm package is a thin distribution layer:

- publish the npm package once
- on install, download the matching GitHub Release binary archive
- expose `section` and `sectiond` through npm bin shims

This keeps the product line clear:

- Rust binaries remain the real product artifacts
- npm remains an install surface
- Section still does not become a JS SDK by accident

## Current Binary Targets

The first npm package should support these release archives:

- `section-<version>-darwin-arm64.tar.gz`
- `section-<version>-darwin-x64.tar.gz`
- `section-<version>-linux-arm64.tar.gz`
- `section-<version>-linux-x64.tar.gz`

Each archive should extract to:

- `bin/section`
- `bin/sectiond`

## Files in This Package

- [PUBLISH_FLOW.md](PUBLISH_FLOW.md)
- [../../../packaging/npm/package.json](../../../packaging/npm/package.json)
- [../../../packaging/npm/README.md](../../../packaging/npm/README.md)
- [../../../packaging/npm/bin/section.js](../../../packaging/npm/bin/section.js)
- [../../../packaging/npm/bin/sectiond.js](../../../packaging/npm/bin/sectiond.js)
- [../../../packaging/npm/lib/runtime.js](../../../packaging/npm/lib/runtime.js)
- [../../../packaging/npm/scripts/postinstall.js](../../../packaging/npm/scripts/postinstall.js)

## Current Boundary

This task prepares the npm install surface and publishable package skeleton.

It does not yet:

- publish the package to npm
- build or attach the release archives automatically
- promise a Windows npm install path before Windows binaries exist
- promise a JavaScript or TypeScript SDK
