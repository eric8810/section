# @eric8810/section

Install the Section CLI binaries through npm.

## Install

```bash
npm install -g @eric8810/section
section --help
sectiond inspect
```

## One-shot execution

```bash
npx --package @eric8810/section section --help
```

## What this package is

This package is an install surface for Section.

It downloads the matching GitHub Release binary archive during `postinstall`, then exposes:

- `section`
- `sectiond`

## What this package is not

This package is not a JavaScript SDK.

The real product artifacts are still the Rust binaries published from:

- <https://github.com/eric8810/section/releases>
