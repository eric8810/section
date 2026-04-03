# Section

Agent-first unified data layer built on [Apache OpenDAL](https://github.com/apache/opendal).

Section targets macOS and Linux together. It mounts multiple storage backends (S3, WebDAV, local filesystem, etc.) as a unified FUSE filesystem where the platform runtime allows it, and exposes the same sources through a CLI for non-mount workflows.

## Features

- **FUSE filesystem** — mount all sources under `/mnt/section/{source_name}/`
- **60+ storage backends** via OpenDAL (S3, WebDAV, Google Drive, Azure Blob, etc.)
- **CLI** — `section ls`, `cat`, `cp`, `rm`, `write`, `exec` across any source
- **Credential encryption** — AES-256-GCM encrypted storage in SQLite
- **JSON output** — `--json` flag for machine-readable output (agent-friendly)
- **POSIX permissions** — uid/gid/mode enforced at the FUSE layer
- **Metadata cache** — TTL-based caching to reduce backend calls
- **Content cache** — LRU eviction cache for file content

## Platform Support

Section is aiming for macOS and Linux together, but the maturity level is not identical across every path yet.

| Capability | Linux | macOS | Notes |
|------------|-------|-------|-------|
| `section-core` / `section-provider` / non-mount CLI | Target platform | Target platform | Covered by the dual-platform CI workflow for non-FUSE paths |
| FUSE mount lifecycle | Primary validation path | Separate validation path | macOS requires macFUSE and platform-specific validation |
| Permission model | POSIX-first implementation | POSIX-like target with runtime differences | Current permission model is still Linux-centric |

Current repo truth:
- cross-platform CLI/core/provider support is an active MVP target
- Linux remains the primary reference path for FUSE behavior
- macOS mount support is being hardened separately instead of being hand-waved as already done

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

# File operations
section ls my-files/
section cat my-files/hello.txt
section cp my-files/doc.pdf work-s3/backup/doc.pdf
echo "hello" | section write my-files/greeting.txt
section rm work-s3/old-file.txt

# Execute a script from a source
section exec my-files/scripts/deploy.sh -- --env prod

# Mount as filesystem
section mount /mnt/section
ls /mnt/section/my-files/
cat /mnt/section/work-s3/report.csv

# Check status
section status
```

## Architecture

```
┌──────────────┐  ┌──────────────┐
│  AI Agent    │  │  Human/CLI   │
└──────┬───────┘  └──────┬───────┘
       │ POSIX fs ops     │ section <cmd>
       ▼                  ▼
┌─────────────────────────────────┐
│         section-fuse            │
│   FUSE filesystem (fuser)       │
│   inode mgmt, permissions       │
├─────────────────────────────────┤
│         section-core            │
│   Router, Cache, Permissions    │
├─────────────────────────────────┤
│       Apache OpenDAL            │
│   S3 │ WebDAV │ fs │ ...       │
└─────────────────────────────────┘
```

### Crate Structure

| Crate | Description |
|-------|-------------|
| `section-core` | Config, path router, permission model, metadata/content cache |
| `section-cli` | CLI binary (`section` command) |
| `section-fuse` | FUSE filesystem daemon |
| `section-provider` | SQLite source store, credential encryption |

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
cargo check -p section-core -p section-provider -p section-cli
cargo test -p section-core -p section-provider
cargo test -p section-cli

# Full workspace / FUSE validation is tracked separately and is not yet the truthful green path on every platform.

# Run with debug logging
RUST_LOG=debug section ls my-files/
```

## License

Apache-2.0
