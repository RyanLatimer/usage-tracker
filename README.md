# usage-tracker

Simple CLI, runnable in any directory, that sums agent usage across all
installed coding harnesses: total **sessions, messages, tokens
(input / output / cache-read / cache-write)** and **total API cost** —
with a per-token-category breakdown **per harness** and **per model**.

Stdlib only. No network. Reads local session stores.

## Scoping

Results are scoped to a directory **including its subdirectories** — a
session counts if the directory it ran in is the scope or nested under it.

```bash
agent-usage                  # current directory + subdirectories
agent-usage --dir ~/Projects/Lighthouse
agent-usage --global         # all directories
agent-usage --json | jq .scope
```

## Run

```bash
# from anywhere
python /home/ryan/Projects/usage-tracker/usage_tracker.py
python /home/ryan/Projects/usage-tracker/usage_tracker.py --json
python /home/ryan/Projects/usage-tracker/usage_tracker.py --harness opencode,codex

# or install once, then run as `agent-usage` in any directory
pip install -e /home/ryan/Projects/usage-tracker
agent-usage
agent-usage --json
```

## Install on another machine (macOS / Linux)

Prebuilt binaries live under `release/` as versioned tarballs
(`agent-usage-<version>-<os>-<arch>.tar.gz` + `SHA256SUMS.txt`),
one per platform — including **macOS Apple Silicon (`darwin-arm64`)**
and Intel (`darwin-x64`). The only runtime dependency is `python3`
(stdlib only); on macOS install it with `brew install python`.

```bash
# on the Mac, from tarballs copied over (scp, AirDrop, USB…):
bash install.sh --local ./release            # picks the right tarball
bash install.sh --local agent-usage-0.1.0-darwin-arm64.tar.gz

# once this repo is on GitHub (tag v* builds the tarballs via CI):
curl -fsSL https://raw.githubusercontent.com/<you>/usage-tracker/main/install.sh | \
  bash -s -- --repo <you>/usage-tracker
```

This installs `agent-usage` (plus a `usage-tracker` symlink) into
`~/.local/bin`. If that isn't on your `PATH`, the installer tells you —
on macOS add `export PATH="$HOME/.local/bin:$PATH"` to `~/.zshrc`.
Verify with `agent-usage --version`.

> First launch on macOS may show a Gatekeeper prompt (the binary isn't
> notarized): right-click → Open, or
> `xattr -d com.apple.quarantine ~/.local/bin/agent-usage`.

## TUI / binary (building from source)

`agent-usage` (alias `usage-tracker`) is a standalone [Bun](https://bun.sh)
binary: an interactive [OpenTUI](https://opentui.com) dashboard over the
python core. `usage_tracker.py` and `pricing.json` are embedded at build time
and unpacked to `~/.cache/agent-usage/<hash>/`; the python core stays stdlib-only.

```bash
cd /home/ryan/Projects/usage-tracker/tui
bun run setup          # once: deps incl. macOS cross-compile bits
bun run install-bin    # build dist/agent-usage → ~/.local/bin/agent-usage (+ usage-tracker symlink)

agent-usage                    # TUI, scoped to cwd
agent-usage --global --harness claude,codex
agent-usage --json             # old CLI output (also --plain, --help, or when piped)
```

Re-run `bun run install-bin` after editing `usage_tracker.py` or `pricing.json`.

## Native Rune app (macOS .app)

`native/` is a native Rust UI built with the
[Rune](https://github.com/anomalyco/rune) graphics library (`dev` rev
`e12e3e4`), using the OpenCode theme's neutral surfaces, blue accent,
Inter typography, and light/dark appearances. The dashboard includes a source
sidebar, date filters, daily cost bars, token totals, source cost shares, and
an uncapped, scrollable model breakdown. Refreshes scan in the background.

Date filters use UTC session start dates for OpenCode, Codex, Claude Code,
and Gemini. Undated sessions remain in All time; daily costs are attributed
to the session's start date rather than individual message times. The All time
chart shows the latest 30 days. API costs are estimates, not subscription bills.

The app embeds `usage_tracker.py` + `pricing.json` at compile time and shells
to `python3` at runtime (same as the TUI), so macOS needs `brew install python`.

```bash
cd native
cargo check          # type-check (Linux needs libxkbcommon-dev to link/test)
cargo run            # run windowed (needs display + GPU)
./pack-mac.sh target/release/agent-usage-rune dist  # local .app (on Mac)
```

Keys: `g` toggle global/dir scope · `r` refresh · `t` theme (Auto/Light/Dark).
Scope and refresh controls are in the header; appearance is in the sidebar.

GitHub Actions (`.github/workflows/native-mac.yml`) builds on every push to
`main`/`master` touching `native/**`:
- `macos-14` → `AgentUsage-darwin-arm64.zip` (Apple Silicon)

The zip contains `AgentUsage.app` (ad-hoc signed, not notarized —
first launch: right-click → Open). Tagged pushes (`v*` / `native-v*`)
attach the zip to the GitHub Release.

Cross-compile for other machines (from Linux, no Mac needed):

```bash
bun run build:mac        # Apple Silicon → dist/agent-usage-darwin-arm64
bun run build:mac-intel  # Intel Mac     → dist/agent-usage-darwin-x64
bun run build:all        # + linux-x64, linux-arm64
bun run dist:mac         # mac builds → release/*.tar.gz + SHA256SUMS.txt
bun run dist             # all platforms → release/
```

Keys: `↑/↓` pick harness · `g` toggle global / directory scope · `t` toggle
light/dark · `r` refresh · `q` quit.

Colors follow the terminal's light/dark theme (detected at startup and on
change); pass `--light` or `--dark` to force one.

## Pricing

Built-in `pricing.json` holds estimated USD-per-1M-token rates, matched by
substring against model ids. Unknown models are marked `*` and cost $0.
Override with your exact contract rates:

```bash
agent-usage --pricing ./my-pricing.json
```

## Data sources

| harness | store |
|---|---|
| opencode | `~/.local/share/opencode/opencode.db` (sessions + message counts + token columns) |
| codex | `~/.codex/sessions/**/*.jsonl` (last `token_count` per session = session total; `input` split into fresh vs cached) |
| claude | `~/.claude/projects/**/*.jsonl` (message `usage` blocks) |
| gemini | `~/.gemini/tmp/**/*.json` / `~/.config/gemini/tmp` (best effort) |
| cursor / aider / grok / copilot | detected; token logs not machine-readable yet, reported as 0 with a note |

Token semantics: `input` = fresh input only (cached portion moved to
`cache_read`), `reasoning` is reported as a subset of `output` and is not
double-charged.
