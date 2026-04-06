# Section Homebrew Distribution

## Direct Recommendation

Use a dedicated tap:

- GitHub repo: `eric8810/homebrew-section`
- tap name for users: `eric8810/section`

That keeps the install story simple:

```bash
brew tap eric8810/section
brew install eric8810/section/section
```

## Why this route

- Homebrew is an install surface, not just a marketing channel
- a dedicated tap is the normal third-party route for Homebrew distribution
- it avoids waiting on `homebrew/core`
- it keeps control of versioning and rollout in the Section release flow

Official references:

- Homebrew tap docs: <https://docs.brew.sh/How-to-Create-and-Maintain-a-Tap>
- Homebrew formula docs: <https://docs.brew.sh/Formula-Cookbook>
- Homebrew tap naming shortcuts: <https://docs.brew.sh/Taps>

## What to publish

The first Homebrew package should install:

- `section`
- `sectiond`

That matches the current product line:

- `section` is the user-facing CLI
- `sectiond` is the runtime/control-plane companion

## Files in this package

- [TAP_SETUP.md](TAP_SETUP.md)
- [../../../../packaging/homebrew/section.rb.template](../../../../packaging/homebrew/section.rb.template)

## Install Story

Recommended install story for docs and release posts:

```bash
brew tap eric8810/section
brew install eric8810/section/section
```

Optional one-line version:

```bash
brew install eric8810/section/section
```

That works because `brew` can auto-tap when using the fully qualified formula path.

## Current Boundary

This task prepares the Homebrew distribution package and formula template.

It does not yet:

- publish the tap repo
- attach bottle automation
- promise `homebrew/core`
