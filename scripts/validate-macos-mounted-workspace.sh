#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

base=$(mktemp -d /tmp/section-macos-validate-XXXXXX)
src="$base/src"
data="$base/data"
mnt="$base/mnt"
cfg="$base/section.toml"

mkdir -p "$src/scripts" "$src/tools" "$src/inbox" "$src/out" "$data" "$mnt"

cat > "$src/scripts/report.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
cat "$dir/../inbox/hello.txt" > "$dir/../out/from-bash.txt"
EOF
chmod +x "$src/scripts/report.sh"

cat > "$src/tools/check.py" <<'EOF'
from pathlib import Path

root = Path(__file__).resolve().parent.parent
text = (root / "inbox" / "hello.txt").read_text()
(root / "out" / "from-python.txt").write_text(text)
print(text.strip())
EOF

printf 'hello from mount\n' > "$src/inbox/hello.txt"

cat > "$cfg" <<EOF
mount_point = "$mnt"
data_dir = "$data"
EOF

cleanup() {
  PATH="$repo_root/target/debug:$PATH" cargo run -q -p section-cli -- --config "$cfg" unmount "$mnt" >/dev/null 2>&1 || true
  rm -rf "$base"
}
trap cleanup EXIT

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This validation script is for macOS hosts only." >&2
  exit 1
fi

if [[ ! -d /Library/Filesystems/macfuse.fs ]]; then
  cat >&2 <<'EOF'
macFUSE is not installed at /Library/Filesystems/macfuse.fs.
Install macFUSE, allow the system extension if prompted, and re-login/reboot before running this validation.
EOF
  exit 1
fi

cargo build -q -p section-cli -p section-fuse
export PATH="$repo_root/target/debug:$PATH"

cargo run -q -p section-cli -- --config "$cfg" source add local --provider fs --opt root="$src" >/dev/null
cargo run -q -p section-cli -- --config "$cfg" mount "$mnt" >/dev/null

mount | grep -F " on $mnt " >/dev/null

ls -l "$mnt/local" >/dev/null
cat "$mnt/local/inbox/hello.txt" >/dev/null

bash "$mnt/local/scripts/report.sh"
grep -F "hello from mount" "$src/out/from-bash.txt" >/dev/null

printf 'written through mount\n' > "$mnt/local/out/from-mounted.txt"
grep -F "written through mount" "$src/out/from-mounted.txt" >/dev/null

printf 'updated from backend\n' > "$src/inbox/hello.txt"
python3 - <<PY
import os
os.getxattr(r"$mnt/local/inbox/hello.txt", "user.section.refresh")
PY

python3 "$mnt/local/tools/check.py" >/dev/null
grep -F "updated from backend" "$src/out/from-python.txt" >/dev/null

cargo run -q -p section-cli -- --config "$cfg" unmount "$mnt" >/dev/null

if mount | grep -F " on $mnt " >/dev/null 2>&1; then
  echo "Unmount did not remove $mnt" >&2
  exit 1
fi

echo VALIDATION_OK
