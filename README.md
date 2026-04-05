# Section

Shared filesystem collaboration layer for humans and agents, built on [Apache OpenDAL](https://github.com/apache/opendal).

Section is being reorganized around a filesystem-first model:

- the mounted workspace is the main working surface
- humans, agents, shell tools, editors, and scripts should operate on the same namespace
- CLI/API remain important, but as the control plane rather than the final collaboration surface

The next architectural center is `sectiond`: a long-lived local core that will own routing, cache, refresh, permissions, and health semantics for both mount adapters and control-plane clients.

## Project Truth

What is true today:

- Linux has a validated FUSE happy path
- macOS and Linux non-mount flows are covered by CI
- S3, WebDAV, and local filesystem backends have real validation coverage
- the current repo is still **pre-sectiond**
- macOS mount validation still depends on the external macFUSE runtime

What is changing now:

- the repo is moving away from a "CLI + FUSE feature bundle" story
- the route map is now `FS-first -> sectiond -> Linux reference adapter -> macOS adapter`

For the current roadmap, see:

- [docs/PRODUCT.md](docs/PRODUCT.md)
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- [docs/SECTIOND.md](docs/SECTIOND.md)
- [docs/PLAN.md](docs/PLAN.md)
- [docs/BACKEND_VALIDATION.md](docs/BACKEND_VALIDATION.md)

## Features

- **Unified mounted namespace** — the target product model is a shared filesystem workspace
- **60+ storage backends** via OpenDAL (S3, WebDAV, Google Drive, Azure Blob, etc.)
- **Control-plane CLI** — source management, status, refresh, and fallback file operations
- **Credential encryption** — AES-256-GCM encrypted storage in SQLite
- **JSON output** — `--json` flag for machine-readable output (agent-friendly)
- **POSIX permissions** — uid/gid/mode enforced at the FUSE layer
- **Metadata cache** — TTL-based caching to reduce backend calls
- **Content cache** — LRU eviction cache for file content

## Platform Support

Section is targeting macOS and Linux together, but the truthful maturity level is still different across paths.

| Capability | Linux | macOS | Notes |
|------------|-------|-------|-------|
| `section-core` / `section-provider` / non-mount CLI | Target platform | Target platform | Covered by the dual-platform CI workflow for non-FUSE paths |
| Mounted shared workspace | Validated reference path | Target path, not fully validated yet | macOS still requires macFUSE plus explicit adapter validation |
| Permission model | Primary reference implementation | Target path with runtime differences | Current semantics are still Linux-first |

Current repo truth:
- cross-platform core/provider/control-plane support is real
- Linux remains the current reference path for mounted-workspace behavior
- macOS mount support is still an explicit adapter track, not a solved parity claim

## Quick Start

```bash
# Build
cargo build --release

# On macOS, install macFUSE before validating mount/unmount.
# On Linux, install a FUSE runtime such as fuse3.

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

# Mounted workspace (the intended primary working surface)
section mount /mnt/section
ls /mnt/section/my-files/
cat /mnt/section/work-s3/report.csv

# Check status
section status
```

## Architecture

Section is moving toward this runtime model:

```
 Humans / Agents / Shell / Editors
                |
      +---------+---------+
      |                   |
  Control Plane       Data Plane
 (CLI / API / GUI)   (mounted namespace)
      |                   |
      +---------+---------+
                |
             sectiond
 route / cache / refresh / permissions / health
                |
    +-----------+------------+
    |                        |
 Linux mount adapter    macOS mount adapter
    |                        |
           Apache OpenDAL
       S3 / WebDAV / fs / ...
```

Current repo truth:

- `sectiond` is the target center, not a finished component yet
- today's crates still reflect a pre-sectiond structure
- Linux mount validation exists already and becomes the reference data-plane path

### Current Crates

| Crate | Description |
|-------|-------------|
| `section-core` | Config, path router, permission model, metadata/content cache |
| `section-cli` | CLI binary (`section` command) |
| `section-fuse` | FUSE filesystem daemon |
| `section-provider` | SQLite source store, credential encryption |
| `sectiond` | Initial shared runtime boundary and future daemon skeleton |

For the target architecture and the migration plan, see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md), [docs/SECTIOND.md](docs/SECTIOND.md), and [docs/PLAN.md](docs/PLAN.md).

## Supported Providers

Any provider supported by [OpenDAL](https://docs.rs/opendal) can be used. Common ones:

| Provider | `--provider` value | Required options |
|----------|--------------------|------------------|
| Local filesystem | `fs` | `root=/path` |
| Amazon S3 | `s3` | `bucket`, `region`, `access_key_id`, `secret_access_key` |
| WebDAV | `webdav` | `endpoint`, `username`, `password` |

See [OpenDAL services](https://docs.rs/opendal/latest/opendal/services/index.html) for the full list.

## JSON Mode

Add `--json` to any command for machine-readable output:

```bash
section source list --json
# [{"name":"my-files","provider":"fs","options":{"root":"/home/user/docs"}}]

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

# Full mounted-workspace validation is tracked separately and is not yet the truthful green path on every platform.

# Run with debug logging
RUST_LOG=debug section ls my-files/
```

## License

Apache-2.0
