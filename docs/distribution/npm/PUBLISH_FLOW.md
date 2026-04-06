# Section npm Publish Flow

## Direct Release Rule

The npm package version must match the GitHub Release version.

Example:

- GitHub tag: `v0.1.0-alpha.1`
- npm package version: `0.1.0-alpha.1`

## Required Release Archives

For each npm-supported platform, attach one binary archive to the GitHub Release:

- `section-<version>-darwin-arm64.tar.gz`
- `section-<version>-darwin-x64.tar.gz`
- `section-<version>-linux-arm64.tar.gz`
- `section-<version>-linux-x64.tar.gz`

Each archive must unpack to:

- `bin/section`
- `bin/sectiond`

## Publish Steps

1. Build `section` and `sectiond` for each supported target.
2. Package each target into the required archive layout.
3. Attach those archives to the GitHub Release for the same version.
4. Update `packaging/npm/package.json` to the same semver.
5. Run:

   ```bash
   cd packaging/npm
   npm pack --dry-run
   ```

6. Publish:

   ```bash
   npm publish --access public
   ```

7. Verify global install:

   ```bash
   npm install -g @eric8810/section
   section --help
   sectiond inspect
   ```

8. Verify one-shot execution:

   ```bash
   npx --package @eric8810/section section --help
   ```

## Practical Boundary

This npm package is intentionally only a distribution layer.

It should stay responsible for:

- install
- download
- bin shims

It should not become responsible for:

- source sync logic
- product runtime logic
- JS API design
