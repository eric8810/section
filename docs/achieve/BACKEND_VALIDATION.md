# Backend Validation

This document records a repeatable local validation path for the first-phase remote providers.

Validated on 2026-04-05:
- S3-compatible backend via `moto_server`
- WebDAV backend via `wsgidav` + `cheroot`
- Linux FUSE mount happy path via Ubuntu 24.04 guest on Lima (`vz`)

Validated flows for both providers:
- `section source add`
- `section source list`
- `section ls`
- `section write`
- `section cat`
- `section cp`
- `section rm`

Validated flows for the Linux FUSE path:
- `section source add`
- `section mount`
- `ls` / `cat` through the mounted path
- write-through via the mounted path
- refresh via the mount xattr after an out-of-band backend mutation
- `section unmount`

## Prerequisites

Create an isolated Python environment:

```bash
python3 -m venv .venv-backend
. .venv-backend/bin/activate
python -m pip install "moto[server,s3]" wsgidav cheroot boto3
```

Build the CLI:

```bash
cargo build -p section-cli
```

## S3 Validation

Start a local S3-compatible endpoint:

```bash
.venv-backend/bin/moto_server -H 127.0.0.1 -p 19000
```

Create a bucket:

```bash
.venv-backend/bin/python - <<'PY'
import boto3
s3 = boto3.client(
    "s3",
    endpoint_url="http://127.0.0.1:19000",
    aws_access_key_id="test",
    aws_secret_access_key="test",
    region_name="us-east-1",
)
s3.create_bucket(Bucket="section-validation")
PY
```

Create a temporary config:

```bash
mkdir -p .tmp/section-s3-data
printf 'data_dir = "%s"\nmount_point = "/tmp/section-backend-mount"\n' \
  "$PWD/.tmp/section-s3-data" > .tmp/section-s3.toml
```

Run the end-to-end flow:

```bash
./target/debug/section --config .tmp/section-s3.toml source add test-s3 \
  --provider s3 \
  --opt endpoint=http://127.0.0.1:19000 \
  --opt region=us-east-1 \
  --opt bucket=section-validation \
  --opt access_key_id=test \
  --opt secret_access_key=test

printf 'hello-s3' | ./target/debug/section --config .tmp/section-s3.toml write test-s3/hello.txt
./target/debug/section --config .tmp/section-s3.toml cat test-s3/hello.txt
./target/debug/section --config .tmp/section-s3.toml ls test-s3/
./target/debug/section --config .tmp/section-s3.toml cp test-s3/hello.txt test-s3/copy.txt
./target/debug/section --config .tmp/section-s3.toml rm test-s3/copy.txt
```

Observed result:
- All commands completed successfully against the remote backend.
- No provider-specific workaround was required beyond the explicit local endpoint.

## WebDAV Validation

Create a WsgiDAV config with basic auth:

```yaml
host: 127.0.0.1
port: 19080
provider_mapping:
  "/": "/ABS/PATH/TO/webdav-root"
http_authenticator:
  accept_basic: true
  accept_digest: false
  default_to_digest: false
simple_dc:
  user_mapping:
    "*":
      section:
        password: section-pass
        description: Section validation user
        roles: []
dir_browser:
  enable: false
verbose: 1
```

Start the server:

```bash
.venv-backend/bin/wsgidav --config .tmp/wsgidav.yaml
```

Create a temporary Section config:

```bash
mkdir -p .tmp/section-webdav-data .tmp/webdav-root
printf 'data_dir = "%s"\nmount_point = "/tmp/section-backend-mount"\n' \
  "$PWD/.tmp/section-webdav-data" > .tmp/section-webdav.toml
```

Run the end-to-end flow:

```bash
./target/debug/section --config .tmp/section-webdav.toml source add test-dav \
  --provider webdav \
  --opt endpoint=http://127.0.0.1:19080 \
  --opt username=section \
  --opt password=section-pass

printf 'hello-dav' | ./target/debug/section --config .tmp/section-webdav.toml write test-dav/hello.txt
./target/debug/section --config .tmp/section-webdav.toml cat test-dav/hello.txt
./target/debug/section --config .tmp/section-webdav.toml ls test-dav/
./target/debug/section --config .tmp/section-webdav.toml cp test-dav/hello.txt test-dav/copy.txt
./target/debug/section --config .tmp/section-webdav.toml rm test-dav/copy.txt
```

Observed result:
- All commands completed successfully against WsgiDAV when using the Cheroot-backed server.
- `Router::build_operator()` now normalizes trailing `/` on WebDAV endpoints, so both `http://127.0.0.1:19080` and `http://127.0.0.1:19080/` are accepted.
- CLI `ls` now filters the collection self-entry that some WebDAV `PROPFIND` responses include.

## Linux FUSE Validation

Validated environment:
- host: macOS 26.3.1 (Apple Silicon)
- guest: Ubuntu 24.04.4 LTS (`aarch64`) under Lima 2.1.1 with the `vz` driver

Guest prerequisites:

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libfuse3-dev curl ca-certificates
curl https://sh.rustup.rs -sSf | sh -s -- -y
```

Linux FUSE requires `allow_other`, so enable it explicitly in the guest:

```bash
sudo sed -i 's/^#user_allow_other/user_allow_other/' /etc/fuse.conf
```

Build the CLI and FUSE daemon:

```bash
cargo build -p section-cli -p section-fuse
```

Create a local fs-backed source and a temporary Section config:

```bash
mkdir -p .tmp/linux-fuse-src/nested .tmp/linux-fuse-data .tmp/linux-fuse-mnt
printf 'hello-from-backend\n' > .tmp/linux-fuse-src/hello.txt
printf 'nested-file\n' > .tmp/linux-fuse-src/nested/file.txt

cat > .tmp/linux-fuse.toml <<EOF
mount_point = "$PWD/.tmp/linux-fuse-mnt"
data_dir = "$PWD/.tmp/linux-fuse-data"
EOF

./target/debug/section --config .tmp/linux-fuse.toml source add my-files \
  --provider fs \
  --opt root="$PWD/.tmp/linux-fuse-src"
```

Run the happy-path validation:

```bash
PATH="$PWD/target/debug:$PATH" ./target/debug/section --config .tmp/linux-fuse.toml mount .tmp/linux-fuse-mnt
mountpoint .tmp/linux-fuse-mnt
ls -l .tmp/linux-fuse-mnt/my-files
cat .tmp/linux-fuse-mnt/my-files/hello.txt

printf 'written-through-mount\n' > .tmp/linux-fuse-mnt/my-files/mounted.txt
cat .tmp/linux-fuse-src/mounted.txt

printf 'updated-from-backend\n' > .tmp/linux-fuse-src/hello.txt
python3 - <<'PY'
import os
print(os.getxattr(".tmp/linux-fuse-mnt/my-files/hello.txt", "user.section.refresh").decode())
PY
cat .tmp/linux-fuse-mnt/my-files/hello.txt

./target/debug/section --config .tmp/linux-fuse.toml unmount .tmp/linux-fuse-mnt
```

Observed result:
- `section mount` created a real FUSE mount and `mountpoint` reported it as active.
- `ls -l` on both CLI and mount path reported the correct file size for `hello.txt` (`19` bytes in the validation fixture), instead of the previous incorrect `0`.
- `cat` through the mount returned the backend content.
- writing `mounted.txt` through the mount persisted the file into the backend directory.
- after an out-of-band backend update, reading the `user.section.refresh` xattr invalidated the mounted view and the next `cat` returned the new content.
- `section unmount` cleanly removed the mount.

## Known Caveats

- Do not use `wsgidav --server=wsgiref` as the validation server. It can hang on the same GET/PUT flows that succeed under Cheroot.
- This validation path is documented local testing, not CI. The current GitHub workflow still has no containerized backend harness for remote-provider checks.
- On Linux, the current mount flow depends on `allow_other`, so `/etc/fuse.conf` must enable `user_allow_other` for a non-root validation user.
