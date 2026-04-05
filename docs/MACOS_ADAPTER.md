# macOS Mount Adapter Plan

## Truthful current state

macOS is part of the target support matrix, but the mounted shared-workspace path is not zero-dependency today.

Current truth:

- non-mount `section` workflows are expected to work on macOS
- the mounted workspace path currently depends on `macFUSE`
- `section-fuse` must also be installed and available in `PATH`
- the full shared-workspace validation for macOS is still tracked separately in `#11`

This means macOS mount support is a productized prerequisite story first, and a fully validated adapter story second.

## Required prerequisites

Before using `section mount` on macOS, the system needs:

1. `section-fuse` installed and available in `PATH`
2. `macFUSE` installed at `/Library/Filesystems/macfuse.fs`
3. any system-extension approval steps completed
4. a re-login or reboot if macFUSE installation requires it

Typical local install path during development:

```bash
cargo install --path crates/section-cli
cargo install --path crates/section-fuse
```

If you are running from a dev checkout instead of `cargo install`, make the built binaries visible:

```bash
export PATH="$PWD/target/debug:$PATH"
```

## CLI preflight expectations

`section mount` now performs an explicit preflight before it tries to spawn `section-fuse`.

On macOS, the preflight checks for:

- an absolute mount path
- `section-fuse` available in `PATH`
- `macFUSE` present at `/Library/Filesystems/macfuse.fs`

If one of these checks fails, `section mount` should stop immediately with a concrete action list instead of failing later with an implicit runtime error.

## Installer / bootstrap story

Short-term productized path:

- install `section-cli`
- install `section-fuse`
- install `macFUSE`
- run `section source add ...`
- run `section mount ...`

This is intentionally explicit. The product should not pretend macOS mount support is dependency-free while it still relies on macFUSE.

## Fallback policy

If macOS prerequisites are missing, the supported fallback is:

- use control-plane commands such as `section source`, `section status`, and `section refresh`
- use non-mount file helpers (`ls`, `cat`, `cp`, `write`, `rm`) as a temporary fallback
- do not claim this fallback is equivalent to the shared mounted-workspace experience

That fallback keeps Section usable, but it is not the end-state collaboration model.

## Criteria for a later native adapter

Revisit a native macOS adapter only if one or more of these become true:

- macFUSE install friction materially blocks adoption
- target users lack permission to install system extensions
- operational support cost around macFUSE becomes too high
- the shared-workspace semantics are already proven and stable enough that replacing the adapter layer is the main remaining problem

Until then, the honest path is to keep the macFUSE-based adapter explicit and well-instrumented.
