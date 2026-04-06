# Section

Cross-platform source/path sync collaboration layer for humans and agents, built on [Apache OpenDAL](https://github.com/apache/opendal).

<img width="1024" height="1024" alt="Gemini_Generated_Image_4x0b3z4x0b3z4x0b" src="https://github.com/user-attachments/assets/f137d183-c005-43a8-89d8-89687fd06bbf" />



## Active Model

Section's active product model is:

- `source/path` is the primary mental model
- a source can bind to a local directory
- files and directories sync into that local tree
- public state stays simple: `ready / syncing / conflict / error`
- `sectiond` is the source/path sync core
- CLI / API / GUI are the control plane
- execution is outside the current project scope

## Active Documents

- [docs/PRODUCT.md](docs/PRODUCT.md)
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- [docs/SYNC_MODEL.md](docs/SYNC_MODEL.md)
- [docs/SECTIOND.md](docs/SECTIOND.md)
- [docs/PLAN.md](docs/PLAN.md)
- [docs/QUICKSTART.md](docs/QUICKSTART.md)
- [docs/USER_MANUAL.md](docs/USER_MANUAL.md)
- [docs/PROMO.md](docs/PROMO.md)
- [docs/release/README.md](docs/release/README.md)

## Current Implementation

The current repo already includes:

- source registry persisted in the provider store
- source to local-root binding
- `.section/root.json` local discovery markers
- source/path sync state persistence
- source/path event persistence
- bidirectional source sync with stale-overwrite conflict detection
- local-path-first control-plane commands:
  - `section source bind`
  - `section source unbind`
  - `section source sync`
  - `section path inspect`
  - `section path compare`
  - `section path resolve`
  - `section watch`

## Repository Focus

The active route in this repo is centered on:

- `section-core`
- `section-provider`
- `section-cli`
- `sectiond`

## Supported Providers

Any provider supported by [OpenDAL](https://docs.rs/opendal) can be used. Common ones:

| Provider | `--provider` value | Required options |
|----------|--------------------|------------------|
| Local filesystem | `fs` | `root=/path` |
| Amazon S3 | `s3` | `bucket`, `region`, `access_key_id`, `secret_access_key` |
| WebDAV | `webdav` | `endpoint`, `username`, `password` |

See [OpenDAL services](https://docs.rs/opendal/latest/opendal/services/index.html) for the full list.

## Development

```bash
cargo check -p section-core -p section-provider -p section-cli -p sectiond
cargo test -p section-core -p section-provider -p sectiond
cargo test -p section-cli
RUST_LOG=debug section source list
```

## License

Apache-2.0
