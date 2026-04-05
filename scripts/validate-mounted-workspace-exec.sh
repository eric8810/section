#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

base=$(mktemp -d /tmp/section-exec-validate-XXXXXX)
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
  if mount | grep -F " on $mnt " >/dev/null 2>&1; then
    fusermount3 -u "$mnt" || true
  fi
  if [[ -n "${fuse_pid:-}" ]] && kill -0 "$fuse_pid" >/dev/null 2>&1; then
    kill "$fuse_pid" || true
    wait "$fuse_pid" || true
  fi
  rm -rf "$base"
}
trap cleanup EXIT

cargo run -q -p section-cli -- --config "$cfg" source add local --provider fs --opt root="$src" >/dev/null
cargo run -q -p section-fuse -- --config "$cfg" --mount-point "$mnt" > "$base/fuse.log" 2>&1 &
fuse_pid=$!

for _ in $(seq 1 100); do
  if mount | grep -F " on $mnt " >/dev/null 2>&1; then
    break
  fi
  if ! kill -0 "$fuse_pid" >/dev/null 2>&1; then
    cat "$base/fuse.log"
    exit 1
  fi
  sleep 0.2
done

mount | grep -F " on $mnt " >/dev/null

bash "$mnt/local/scripts/report.sh"
grep -F "hello from mount" "$src/out/from-bash.txt" >/dev/null

printf 'updated from backend\n' > "$src/inbox/hello.txt"
python3 - <<PY
import os
os.getxattr(r"$mnt/local/inbox/hello.txt", "user.section.refresh")
PY

python3 "$mnt/local/tools/check.py" >/dev/null
grep -F "updated from backend" "$src/out/from-python.txt" >/dev/null

echo VALIDATION_OK
