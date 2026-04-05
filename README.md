# Section

Cross-platform source/path sync collaboration layer for humans and agents, built on [Apache OpenDAL](https://github.com/apache/opendal).

Section is pivoting away from a mount-first product model toward a source/path-first sync model:

- `source/path` remains the primary product mental model
- sync and conflict become state on sources and paths
- a local bound directory is a manifestation of a source, not a new top-level product object
- public path state should stay simple: `ready / syncing / conflict / error`
- CLI/API remain the control plane
- execution semantics are treated as a separate runtime problem, not something the filesystem layer can magically unify

The next architectural center is `sectiond`: a long-lived local core that will own source registry, local sync bindings, path state, local-presence detail, refresh, conflicts, and health semantics for both control-plane clients and future adapters.

## Project Truth

What is true today:

- Linux has a validated FUSE happy path
- macOS and Linux non-mount flows are covered by CI
- S3, WebDAV, and local filesystem backends have real validation coverage
- the current repo is still **pre-sectiond**
- the repo still reflects the older mount-first route map more than the new source/path sync route

What is changing now:

- the repo is moving away from a "CLI + FUSE feature bundle" story
- the new route map is `source/path sync state -> sectiond sync core -> execution contract -> future adapters`

For the current roadmap, see:

- [docs/PRODUCT.md](docs/PRODUCT.md)
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- [docs/SYNC_MODEL.md](docs/SYNC_MODEL.md)
- [docs/SECTIOND.md](docs/SECTIOND.md)
- [docs/PLAN.md](docs/PLAN.md)
- [docs/BACKEND_VALIDATION.md](docs/BACKEND_VALIDATION.md)

## Features

- **Source/path mainline** — sources and paths remain the primary product model
- **Simple status model** — user-facing path state stays at `ready / syncing / conflict / error`
- **Local directory sync** — sources and paths can be synced into a truthful local directory view
- **60+ storage backends** via OpenDAL (S3, WebDAV, Google Drive, Azure Blob, etc.)
- **Control-plane CLI** — source management, path operations, sync state, refresh, and fallback file operations
- **Credential encryption** — AES-256-GCM encrypted storage in SQLite
- **JSON output** — `--json` flag for machine-readable output (agent-friendly)
- **Mount groundwork** — Linux FUSE and macOS/macFUSE groundwork remain available as future/advanced tracks
- **Metadata cache** — TTL-based caching to reduce backend calls
- **Content cache** — LRU eviction cache for file content

## Platform Support

Section is targeting macOS, Linux, and Windows as a source/path sync product, but the current repo maturity still reflects earlier mount-focused work.

| Capability | Linux | macOS | Windows | Notes |
|------------|-------|-------|---------|-------|
| `section-core` / `section-provider` / non-mount CLI | Target platform | Target platform | Target platform | current CI covers Linux/macOS non-FUSE paths |
| Source/path sync mainline | Pivot target | Pivot target | Pivot target | not implemented end-to-end yet |
| Mount / adapter path | Advanced track | Advanced track | Advanced track | no longer the main product path |

Current repo truth:
- cross-platform core/provider/control-plane support is real
- Linux remains the place where the old mounted-workspace path is most validated
- source/path sync is now the product mainline, but still needs dedicated implementation work
- macOS mount details remain documented in [docs/MACOS_ADAPTER.md](docs/MACOS_ADAPTER.md) as a future/advanced path, not the default user journey

## Quick Start

```bash
# Build
cargo build --release

# Interactive setup
section init

# Or manually add a source
section source add my-files --provider fs --opt root=/home/user/documents
section source add work-s3 --provider s3 \
  --opt bucket=my-bucket \
  --opt region=us-east-1 \
  --opt access_key_id=AKIA... \
  --opt secret_access_key=...

# List sources
section source list

# Control-plane and fallback file operations
section ls my-files/
section ls -l my-files/
section cat my-files/hello.txt
section cp my-files/doc.pdf work-s3/backup/doc.pdf
section cp ./local.txt my-files/local.txt
section cp my-files/report.pdf ./report.pdf
section cp -r my-files/docs/ work-s3/docs/
echo "hello" | section write my-files/greeting.txt
section rm work-s3/old-file.txt

# Supplementary exec helper
section exec my-files/scripts/deploy.sh -- --env prod

# Check status
section status
```

Current and target source/path sync semantics are documented in [docs/SYNC_MODEL.md](docs/SYNC_MODEL.md).
Old mounted-workspace execute/scripting semantics remain documented in [docs/EXECUTION_MODEL.md](docs/EXECUTION_MODEL.md) as groundwork/history.
macOS mount prerequisite / preflight / fallback policy remains documented in [docs/MACOS_ADAPTER.md](docs/MACOS_ADAPTER.md) for the advanced path.

## Architecture

Section is moving toward this runtime model:

```
 Humans / Agents / Shell / Editors
                |
       Local Source Trees
                |
             sectiond
 route / cache / sync / local state / conflicts / health
                |
      +---------+---------+
      |                   |
 Control Plane      Future Adapters
 (CLI / API / GUI) (FUSE / File Provider /
                    CFAPI / SMB export)
                |
           Apache OpenDAL
       S3 / WebDAV / fs / ...
```

Current repo truth:

- `sectiond` is the target center, not a finished component yet
- today's crates still reflect a pre-sectiond and pre-source-path-sync structure
- old Linux mount validation exists already, but is no longer the default product path

### Current Crates

| Crate | Description |
|-------|-------------|
| `section-core` | Config, path router, permission model, metadata/content cache |
| `section-cli` | CLI binary (`section` command) |
| `section-fuse` | FUSE filesystem daemon |
| `section-provider` | SQLite source store, credential encryption |
| `sectiond` | Initial shared runtime boundary and future daemon skeleton |

For the target architecture and the migration plan, see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md), [docs/SYNC_MODEL.md](docs/SYNC_MODEL.md), [docs/SECTIOND.md](docs/SECTIOND.md), and [docs/PLAN.md](docs/PLAN.md).

## Supported Providers

Any provider supported by [OpenDAL](https://docs.rs/opendal) can be used. Common ones:

| Provider | `--provider` value | Required options |
|----------|--------------------|------------------|
| Local filesystem | `fs` | `root=/path` |
| Amazon S3 | `s3` | `bucket`, `region`, `access_key_id`, `secret_access_key` |
| WebDAV | `webdav` | `endpoint`, `username`, `password` |

See [OpenDAL services](https://docs.rs/opendal/latest/opendal/services/index.html) for the full list.

## JSON Mode

Add `--json` to any command for machine-readable output. Control-plane commands (`source`, `status`, `refresh`) now report the `sectiond` view of the world rather than a raw CLI-local snapshot:

```bash
section source list --json
# [{"name":"my-files","provider":"fs","origin":"provider_store","metadata_ttl_secs":60,"content_ttl_secs":300,"options":{"root":"/home/user/docs"}}]

section ls --json my-files/
# [{"name":"hello.txt","type":"file","size":13},{"name":"docs/","type":"directory"}]

section status --json
# {"mount":{"path":"/mnt/section","active":false},"sources":[{"name":"my-files","provider":"fs","connected":true}]}
```

## Configuration

Config file location: `~/.config/section/config.toml` (or `$XDG_CONFIG_HOME/section/config.toml`)

```toml
data_dir = "~/.local/share/section"
mount_point = "/mnt/section"
```

Sources are stored in SQLite at `{data_dir}/section.db` with credentials encrypted via AES-256-GCM (key at `{data_dir}/section.key`).

## Development

```bash
# Non-FUSE checks that are intended to stay green on both macOS and Linux
cargo check -p section-core -p section-provider -p section-cli -p sectiond
cargo test -p section-core -p section-provider -p sectiond
cargo test -p section-cli

# Old mounted-workspace validation remains available as groundwork/history
scripts/validate-mounted-workspace-exec.sh

# Run with debug logging
RUST_LOG=debug section ls my-files/
```

## License

Apache-2.0
