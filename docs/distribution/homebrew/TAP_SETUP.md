# Homebrew Tap Setup

## Target

Create a dedicated tap repository:

- `eric8810/homebrew-section`

Users will consume it as:

- `eric8810/section`

## Repo Structure

The tap repo should contain:

```text
Formula/
  section.rb
```

## Initial Setup

Use the Homebrew helper:

```bash
brew tap-new eric8810/homebrew-section
```

Then copy the prepared formula template into:

```text
Formula/section.rb
```

## Release Update Flow

For each new Section release:

1. create the GitHub release/tag in `eric8810/section`
2. compute the source tarball SHA256
3. update `version` and `sha256` in `Formula/section.rb`
4. commit and push the tap repo
5. verify install from a clean machine

## Recommended first version

Use:

- `v0.1.0-alpha.1`

And keep the formula honest about the current project line:

- source/path sync
- local-root binding
- control-plane CLI
- no promise of broad mount-first behavior

## Verification Commands

After publishing the tap:

```bash
brew update
brew tap eric8810/section
brew install eric8810/section/section
section --help
sectiond --help
```

## Next Upgrade Step

Once install is stable, the next Homebrew-specific improvement is:

- bottle automation

That is not required for the first distribution cut.
