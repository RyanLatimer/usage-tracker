#!/usr/bin/env bash
# Install agent-usage (interactive usage dashboard + CLI) on macOS or Linux.
#
# From a GitHub release (needs USAGE_TRACKER_REPO or --repo):
#   curl -fsSL https://raw.githubusercontent.com/<you>/usage-tracker/main/install.sh | bash
#   curl -fsSL .../install.sh | bash -s -- --version 0.1.0
#
# From tarballs built with `bun run dist` (e.g. copied over with scp/AirDrop):
#   bash install.sh --local ./release
#   bash install.sh --local ./agent-usage-0.1.0-darwin-arm64.tar.gz
#
# Only runtime dependency is python3 (stdlib only): `brew install python` on macOS.
set -euo pipefail

REPO="${USAGE_TRACKER_REPO:-}"
VERSION="${USAGE_TRACKER_VERSION:-latest}"
PREFIX="${PREFIX:-$HOME/.local/bin}"
LOCAL=""
REPO_FLAG=""

usage() {
  sed -n '2,/^set /p' "$0" | sed 's/^# \{0,1\}//'
  echo "Options: --local <file|dir>  --repo <owner/name>  --version <tag|latest>  --prefix <dir>"
}

while [ $# -gt 0 ]; do
  case "$1" in
    --local) LOCAL="${2:?--local needs a file or directory}"; shift 2 ;;
    --repo) REPO_FLAG="${2:?--repo needs owner/name}"; shift 2 ;;
    --version) VERSION="$2"; shift 2 ;;
    --prefix) PREFIX="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown flag: $1" >&2; usage >&2; exit 1 ;;
  esac
done
[ -n "$REPO_FLAG" ] && REPO="$REPO_FLAG"

os="$(uname -s | tr '[:upper:]' '[:lower:]')"
case "$(uname -m)" in
  arm64|aarch64) arch="arm64" ;;
  x86_64) arch="x64" ;;
  *) echo "unsupported CPU: $(uname -m)" >&2; exit 1 ;;
esac
case "$os" in
  darwin) plat="darwin-${arch}" ;;
  linux) plat="linux-${arch}" ;;
  *) echo "unsupported OS: $(uname -s) (macOS and Linux only)" >&2; exit 1 ;;
esac

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

if [ -n "$LOCAL" ]; then
  if [ -d "$LOCAL" ]; then
    asset="$(ls "$LOCAL"/agent-usage-*-"${plat}".tar.gz 2>/dev/null | head -n1 || true)"
    [ -n "$asset" ] || { echo "no *-${plat}.tar.gz in $LOCAL" >&2; exit 1; }
  else
    asset="$LOCAL"
  fi
  echo "installing from $asset"
  tar -xzf "$asset" -C "$work"
else
  [ -n "$REPO" ] || { echo "set --repo <owner/name> or USAGE_TRACKER_REPO (or use --local)" >&2; exit 1; }
  if [ "$VERSION" = "latest" ]; then
    base="https://github.com/${REPO}/releases/latest/download"
  else
    base="https://github.com/${REPO}/releases/download/${VERSION}"
  fi
  echo "downloading agent-usage ${VERSION} for ${plat}"
  curl -fsSL -o "$work/pkg.tar.gz" "${base}/agent-usage-${VERSION#v}-${plat}.tar.gz"
  if curl -fsSL -o "$work/SHA256SUMS.txt" "${base}/SHA256SUMS.txt" 2>/dev/null; then
    (cd "$work" && mv pkg.tar.gz "$(grep "${plat}.tar.gz" SHA256SUMS.txt | head -n1 | awk '{print $2}')")
    (cd "$work" && (sha256sum -c --status SHA256SUMS.txt 2>/dev/null || shasum -a 256 -c SHA256SUMS.txt))
    echo "checksum ok"
  fi
  tar -xzf "$work"/agent-usage-*.tar.gz -C "$work"
fi

mkdir -p "$PREFIX"
install -m 755 "$work/agent-usage" "$PREFIX/agent-usage"
ln -sf agent-usage "$PREFIX/usage-tracker"

if ! command -v python3 >/dev/null; then
  echo "warning: python3 not found — the binary needs it at runtime." >&2
  if [ "$os" = "darwin" ]; then echo "  macOS: brew install python" >&2
  else echo "  Debian/Ubuntu: sudo apt install python3" >&2; fi
fi

"$PREFIX/agent-usage" --version
case ":$PATH:" in
  *":$PREFIX:"*) ;;
  *) echo "note: $PREFIX is not on PATH — add: export PATH=\"$PREFIX:\$PATH\"" >&2 ;;
esac
echo "installed to $PREFIX/agent-usage (+ usage-tracker)"
