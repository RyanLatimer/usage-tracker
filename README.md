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

## TUI / binary

`agent-usage` (alias `usage-tracker`) is a standalone [Bun](https://bun.sh)
binary: an interactive [OpenTUI](https://opentui.com) dashboard over the
python core. `usage_tracker.py` and `pricing.json` are embedded at build time
and unpacked to `~/.cache/agent-usage/<hash>/`; the python core stays stdlib-only.

```bash
cd /home/ryan/Projects/usage-tracker/tui
bun install            # once
bun run install-bin    # build dist/agent-usage → ~/.local/bin/agent-usage (+ usage-tracker symlink)

agent-usage                    # TUI, scoped to cwd
agent-usage --global --harness claude,codex
agent-usage --json             # old CLI output (also --plain, --help, or when piped)
```

Re-run `bun run install-bin` after editing `usage_tracker.py` or `pricing.json`.

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
