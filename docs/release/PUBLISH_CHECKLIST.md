# GitHub Release Publish Checklist

## Before draft

- repo `master` is green
- release version is chosen
- release title is chosen
- release notes draft is reviewed
- Quick Start and User Manual links work
- launch card and workflow card are ready
- demo recording is ready or explicitly deferred

## Draft release

1. create tag:
   - suggested first tag: `v0.1.0-alpha.1`
2. open GitHub Releases
3. create a new release from the tag
4. paste the body from [GITHUB_RELEASE_DRAFT.md](GITHUB_RELEASE_DRAFT.md)
5. attach or link the assets from [ASSET_CHECKLIST.md](ASSET_CHECKLIST.md)

## Release-page sanity check

- title is readable to first-time visitors
- first paragraph describes the product without jargon overload
- install path is not missing
- Quick Start link is visible
- non-goals are honest
- images render correctly

## Publish

- publish the GitHub Release
- immediately verify the public page while logged out

## After publish

- use the release page as the canonical link for:
  - Show HN
  - community posts
  - later Product Hunt materials

## Do not do this

- do not publish with only architecture language and no user-facing entry point
- do not promise broader install surfaces before tasks `#9` / `#10` land
- do not oversell mount semantics or cross-platform execution guarantees
