#!/usr/bin/env bash
# Pack a macOS .app around the compiled agent-usage-rune binary.
# Usage: native/pack-mac.sh <binary> <output-dir> [AppName] [Identifier] [Version]
# Example (CI): native/pack-mac.sh target/release/agent-usage-rune dist "Agent Usage"
set -euo pipefail

BIN="${1:?path to agent-usage-rune binary}"
OUT="${2:?output dir}"
APP_NAME="${3:-Agent Usage}"
IDENT="${4:-com.local.agent-usage-rune}"
VERSION="${5:-0.1.0}"

# "Agent Usage" -> "AgentUsage.app" (spaces break some tooling; display name stays spaced)
APP_BASE="$(echo "$APP_NAME" | tr -d ' ')"
APP_DIR="$OUT/$APP_BASE.app"

rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"

cp "$BIN" "$APP_DIR/Contents/MacOS/agent-usage-rune"
chmod +x "$APP_DIR/Contents/MacOS/agent-usage-rune"

# Info.plist from template with identifier/version stamped in
sed -e "s/com.local.agent-usage-rune/$IDENT/" \
    -e "s/<string>0.1.0<\/string>/<string>$VERSION<\/string>/g" \
    -e "s/<string>Agent Usage<\/string>/<string>$APP_NAME<\/string>/g" \
    "$(dirname "$0")/Info.plist" > "$APP_DIR/Contents/Info.plist"

# Ad-hoc sign so Gatekeeper lets it run locally without a paid cert.
# (Not notarized — first launch may need right-click → Open.)
if command -v codesign >/dev/null 2>&1; then
  codesign --force --deep --sign - "$APP_DIR" || true
fi

echo "wrote $APP_DIR"
