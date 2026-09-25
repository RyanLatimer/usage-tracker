#!/usr/bin/env bash
# Package compiled per-platform binaries (dist/agent-usage-<os>-<arch>)
# into versioned tarballs + a checksum file under release/.
#
#   bun run dist        # all platforms (needs cross-compile, fine on Linux)
#   bun run dist:mac    # macOS Apple Silicon + Intel only
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
VER="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' "$ROOT/pyproject.toml")"
if [ -z "$VER" ]; then
  echo "dist.sh: could not read version from pyproject.toml" >&2
  exit 1
fi

OUT="$ROOT/release"
mkdir -p "$OUT"

# Stage under the repo, not /tmp (/tmp may be small or quota-limited).
STAGE_ROOT="$ROOT/.tmp-dist"
mkdir -p "$STAGE_ROOT"

count=0
for bin in "$ROOT"/dist/agent-usage-linux-* "$ROOT"/dist/agent-usage-darwin-*; do
  [ -f "$bin" ] || continue
  plat="${bin##*/agent-usage-}"
  stage="$(mktemp -d -p "$STAGE_ROOT")"
  cp "$bin" "$stage/agent-usage"
  chmod +x "$stage/agent-usage"
  tar -czf "$OUT/agent-usage-${VER}-${plat}.tar.gz" -C "$stage" agent-usage
  rm -rf "$stage"
  echo "packaged agent-usage-${VER}-${plat}.tar.gz"
  count=$((count + 1))
done

if [ "$count" -eq 0 ]; then
  echo "dist.sh: no per-platform binaries in dist/ — run bun run build:all first" >&2
  exit 1
fi

(cd "$OUT" && sha256sum ./*.tar.gz > SHA256SUMS.txt)
echo "wrote $OUT/SHA256SUMS.txt"
