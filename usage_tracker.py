#!/usr/bin/env python3
"""agent-usage: measure agent usage across installed coding harnesses.

Reads local session stores (no network) and prints summed sessions,
messages, tokens (input / output / cache_read / cache_write) and
estimated API cost, broken down per harness and per model.

Results are scoped to a directory (default: current directory) including
its subdirectories — a session counts if the directory it ran in is the
scope or nested under it. Pass --global for all directories.

Supported harnesses (best-effort, missing stores are skipped):
  - opencode  (~/.local/share/opencode/opencode.db, session.directory)
  - codex     (~/.codex/sessions/**/*.jsonl, session_meta cwd)
  - claude    (~/.claude/projects/**/*.jsonl, claude-code format)
  - gemini    (~/.gemini/tmp/**/*.json)
  - cursor    (~/.config/Cursor / ~/.cursor)
  - aider     (./.aider* / ~/.aider*)
  - grok      (~/.grok, ~/.xai, ~/.config/grok)
  - t3-code   (flows through opencode/codex stores; reported separately if found)

Usage:
  python usage_tracker.py [--global] [--dir PATH] [--json]
                          [--harness opencode,codex] [--pricing pricing.json]
  agent-usage
  agent-usage --global --json
"""
from __future__ import annotations

import argparse
import collections
import glob
import json
import os
import sqlite3
import sys
from dataclasses import dataclass, field
from pathlib import Path

HOME = Path.home()

# ---------------------------------------------------------------- pricing

# Prices are USD per 1M tokens. These are estimates for common 2026 models;
# override with --pricing <file> (same schema) for exact billing.
DEFAULT_PRICING = {
    # OpenAI GPT-5 family (approx, per 1M)
    "gpt-5.6-terra": {"input": 2.50, "output": 20.00, "cache_read": 0.25, "cache_write": 2.50},
    "gpt-5.6-luna": {"input": 1.25, "output": 10.00, "cache_read": 0.125, "cache_write": 1.25},
    "gpt-5": {"input": 1.25, "output": 10.00, "cache_read": 0.125, "cache_write": 1.25},
    # OpenAI GPT-6 family (models.dev, per 1M)
    "gpt-6-astra": {"input": 10.00, "output": 50.00, "cache_read": 1.00, "cache_write": 12.50},
    "gpt-6-sol": {"input": 2.00, "output": 10.00, "cache_read": 0.20, "cache_write": 2.50},
    "gpt-6-luna": {"input": 0.10, "output": 0.50, "cache_read": 0.01, "cache_write": 0.125},
    "gpt-6": {"input": 2.00, "output": 10.00, "cache_read": 0.20, "cache_write": 2.50},
    "gpt-4": {"input": 2.50, "output": 10.00, "cache_read": 0.25, "cache_write": 2.50},
    "o3": {"input": 2.00, "output": 8.00, "cache_read": 0.50, "cache_write": 2.00},
    # Anthropic Claude (approx)
    "claude": {"input": 3.00, "output": 15.00, "cache_read": 0.30, "cache_write": 3.75},
    # Google Gemini (approx)
    "gemini": {"input": 1.25, "output": 5.00, "cache_read": 0.125, "cache_write": 1.25},
    # xAI Grok (approx)
    "grok": {"input": 2.00, "output": 10.00, "cache_read": 0.20, "cache_write": 2.00},
    # Meta Muse Spark (models.dev, per provider model id; canonical
    # meta/muse-spark-1.3 entry covers both tiers — match contributor first).
    # Contributor tier (incl. "-free" variants, billed here at paid rates):
    # $0.10 in / $0.20 out / $0.002 cache_read per 1M.
    "muse-spark-1.3-contributor": {"input": 0.10, "output": 0.20, "cache_read": 0.002, "cache_write": 0.10},
    "muse-spark-1.2-contributor": {"input": 0.10, "output": 0.20, "cache_read": 0.002, "cache_write": 0.10},
    "muse-spark-contributor": {"input": 0.10, "output": 0.20, "cache_read": 0.002, "cache_write": 0.10},
    # Standard tier: $1.25 in / $4.25 out / $0.15 cache_read per 1M.
    "muse-spark-1.3": {"input": 1.25, "output": 4.25, "cache_read": 0.15, "cache_write": 1.25},
    "muse-spark-1.2": {"input": 1.25, "output": 4.25, "cache_read": 0.15, "cache_write": 1.25},
    "muse-spark-1.1": {"input": 1.25, "output": 4.25, "cache_read": 0.15, "cache_write": 1.25},
    "muse-spark": {"input": 1.25, "output": 4.25, "cache_read": 0.15, "cache_write": 1.25},
}

PRICING_FILE = Path(__file__).with_name("pricing.json")


def load_pricing(path: str | None) -> dict:
    pricing = dict(DEFAULT_PRICING)
    candidates = []
    if path:
        candidates.append(Path(path))
    candidates.append(PRICING_FILE)
    for cand in candidates:
        if cand and cand.exists():
            try:
                extra = json.loads(cand.read_text())
                if isinstance(extra, dict):
                    pricing.update(extra)
            except Exception as e:
                print(f"warning: could not read pricing file {cand}: {e}", file=sys.stderr)
            break
    return pricing


def _strip_free_suffix(model: str) -> str:
    """Strip a trailing free-tier marker so free ids price at paid rates.

    Covers `-free` (e.g. `muse-spark-1.3-contributor-free`), `:free`
    (OpenRouter style), and ` (free)` / ` free` variants. Only strips a
    suffix — a bare `free` elsewhere in the id is left alone.
    """
    low = model.lower()
    for suffix in ("-free", ":free", " (free)", "[free]"):
        if low.endswith(suffix):
            return model[: -len(suffix)].rstrip()
    if low.endswith(" free"):
        return model[: -len(" free")].rstrip()
    return model


def price_for(model: str, pricing: dict) -> dict | None:
    if model in pricing:
        return pricing[model]
    base = _strip_free_suffix(model)
    if base != model and base in pricing:
        return pricing[base]
    # Longest-key substring match so specific tiers win, e.g.
    # "muse-spark-1.3-contributor" beats "muse-spark-1.3" beats "muse-spark"
    # even though all three live under one models.dev canonical entry.
    # Match against the free-stripped id so `-free` variants price at the
    # normal non-free API cost.
    low = base.lower()
    best: dict | None = None
    best_len = -1
    for key, val in pricing.items():
        kl = key.lower()
        if kl in low or low in kl:
            if len(kl) > best_len:
                best = val
                best_len = len(kl)
    return best


def estimate_cost(model: str, inp: int, out: int, cr: int, cw: int, pricing: dict) -> tuple[float, bool]:
    """Return (cost_usd, priced). Unpriced models cost 0 and priced=False."""
    p = price_for(model, pricing)
    if p is None:
        return 0.0, False
    cost = (
        inp * p.get("input", 0) / 1e6
        + out * p.get("output", 0) / 1e6
        + cr * p.get("cache_read", 0) / 1e6
        + cw * p.get("cache_write", 0) / 1e6
    )
    return cost, True


# ---------------------------------------------------------------- records

@dataclass
class ModelStats:
    sessions: int = 0
    messages: int = 0
    input_tokens: int = 0
    output_tokens: int = 0
    cache_read: int = 0
    cache_write: int = 0
    reasoning_tokens: int = 0
    cost_usd: float = 0.0
    recorded_cost_usd: float = 0.0
    unpriced: bool = False

    def add(self, other: "ModelStats") -> None:
        self.sessions += other.sessions
        self.messages += other.messages
        self.input_tokens += other.input_tokens
        self.output_tokens += other.output_tokens
        self.cache_read += other.cache_read
        self.cache_write += other.cache_write
        self.reasoning_tokens += other.reasoning_tokens
        self.cost_usd += other.cost_usd
        self.recorded_cost_usd += other.recorded_cost_usd
        self.unpriced = self.unpriced or other.unpriced


@dataclass
class HarnessResult:
    harness: str
    store: str
    found: bool
    per_model: dict[str, ModelStats] = field(default_factory=dict)
    note: str = ""
    out_of_scope: int = 0
    unattributed: int = 0

    @property
    def total(self) -> ModelStats:
        t = ModelStats()
        for s in self.per_model.values():
            t.add(s)
        return t


# ---------------------------------------------------------------- scoping

def normalize_dir(p: str | os.PathLike | None) -> str | None:
    if not p:
        return None
    try:
        return os.path.realpath(os.path.normpath(os.path.expanduser(str(p))))
    except Exception:
        return os.path.normpath(str(p))


def resolve_scope(dir_arg: str | None, global_flag: bool) -> str | None:
    """Return the scope dir (normalized absolute path) or None for global."""
    if global_flag:
        return None
    base = dir_arg if dir_arg else os.getcwd()
    return normalize_dir(os.path.abspath(base))


def in_scope(sess_dir: str | os.PathLike | None, scope: str | None) -> bool | None:
    """True if session counts, False if out of scope, None if dir unknown.

    scope=None (global) counts everything, including unknown dirs.
    """
    if scope is None:
        return True
    s = normalize_dir(sess_dir)
    if s is None:
        return None
    if s == scope:
        return True
    return s.startswith(scope + os.sep)


# ---------------------------------------------------------------- adapters

def parse_opencode_model(raw: str) -> str:
    try:
        m = json.loads(raw) if raw else {}
        mid = m.get("id", "unknown")
        pid = m.get("providerID", "")
        if pid and pid not in mid:
            return f"{pid}/{mid}"
        return mid or "unknown"
    except Exception:
        return raw or "unknown"


def collect_opencode(pricing: dict, scope: str | None) -> HarnessResult:
    db = HOME / ".local" / "share" / "opencode" / "opencode.db"
    res = HarnessResult("opencode", str(db), found=db.exists())
    if not db.exists():
        res.note = "no store found"
        return res
    try:
        con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
        cur = con.cursor()
        cur.execute(
            "SELECT id, model, cost, tokens_input, tokens_output,"
            " tokens_cache_read, tokens_cache_write, tokens_reasoning, directory FROM session"
        )
        sessions = cur.fetchall()
        cur.execute("SELECT session_id, COUNT(*) FROM message GROUP BY session_id")
        msg_counts = dict(cur.fetchall())
        con.close()
    except Exception as e:
        res.note = f"read error: {e}"
        return res
    for sid, raw_model, cost, ti, to, cr, cw, tr, directory in sessions:
        verdict = in_scope(directory, scope)
        if verdict is None:
            res.unattributed += 1
            if scope is not None:
                continue
        elif not verdict:
            res.out_of_scope += 1
            continue
        model = parse_opencode_model(raw_model or "")
        st = res.per_model.setdefault(model, ModelStats())
        st.sessions += 1
        st.messages += msg_counts.get(sid, 0)
        st.input_tokens += ti or 0
        st.output_tokens += to or 0
        st.cache_read += cr or 0
        st.cache_write += cw or 0
        st.reasoning_tokens += tr or 0
        st.recorded_cost_usd += cost or 0.0
        est, priced = estimate_cost(model, ti or 0, to or 0, cr or 0, cw or 0, pricing)
        # Prefer recorded cost when pricing is free/unknown but a real charge exists.
        st.cost_usd += max(est, cost or 0.0) if not priced else est
        if not priced and not (cost or 0):
            st.unpriced = True
    return res


def collect_codex(pricing: dict, scope: str | None) -> HarnessResult:
    root = HOME / ".codex" / "sessions"
    res = HarnessResult("codex", str(root), found=root.exists())
    if not root.exists():
        res.note = "no store found"
        return res
    files = sorted(root.rglob("*.jsonl"))
    if not files:
        res.note = "no sessions"
        return res
    for path in files:
        model_counter: collections.Counter = collections.Counter()
        cwd_counter: collections.Counter = collections.Counter()
        meta_cwd: str | None = None
        messages = 0
        last_total = None
        try:
            with open(path, errors="replace") as f:
                for line in f:
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        obj = json.loads(line)
                    except Exception:
                        continue
                    t = obj.get("type")
                    if t == "session_meta":
                        meta_cwd = obj.get("payload", {}).get("cwd") or meta_cwd
                    elif t == "turn_context":
                        m = obj.get("payload", {}).get("model", "unknown")
                        model_counter[m or "unknown"] += 1
                        cwd = obj.get("payload", {}).get("cwd")
                        if cwd:
                            cwd_counter[cwd] += 1
                    elif t == "response_item":
                        p = obj.get("payload", {})
                        if p.get("type") == "message" and p.get("role") in ("user", "assistant"):
                            messages += 1
                    elif t == "event_msg":
                        pay = obj.get("payload", {})
                        if pay.get("type") == "token_count":
                            last_total = (pay.get("info") or {}).get("total_token_usage")
        except Exception as e:
            res.note = f"partial read error ({e})"
            continue
        sess_dir = meta_cwd or (cwd_counter.most_common(1)[0][0] if cwd_counter else None)
        verdict = in_scope(sess_dir, scope)
        if verdict is None:
            res.unattributed += 1
            if scope is not None:
                continue
        elif not verdict:
            res.out_of_scope += 1
            continue
        model = model_counter.most_common(1)[0][0] if model_counter else "unknown"
        st = res.per_model.setdefault(model, ModelStats())
        st.sessions += 1
        st.messages += messages
        if last_total:
            inp = int(last_total.get("input_tokens", 0) or 0)
            cached = int(last_total.get("cached_input_tokens", 0) or 0)
            cw = int(last_total.get("cache_write_input_tokens", 0) or 0)
            out = int(last_total.get("output_tokens", 0) or 0)
            rs = int(last_total.get("reasoning_output_tokens", 0) or 0)
            # input_tokens includes cached tokens; report fresh input separately
            # so category columns sum without double counting.
            st.input_tokens += max(inp - cached - cw, 0)
            st.cache_read += cached
            st.cache_write += cw
            st.output_tokens += out
            st.reasoning_tokens += rs
            est, priced = estimate_cost(model, inp, out, cached, cw, pricing)
            st.cost_usd += est
            if not priced:
                st.unpriced = True
        else:
            st.unpriced = True
    return res


def decode_claude_slug(slug: str) -> str | None:
    """Best-effort decode of ~/.claude/projects/<slug> back to a path.

    Claude encodes an absolute path by replacing '/' with '-',
    e.g. '-home-ryan-Projects-foo'. Ambiguous with real dashes but
    good enough as a fallback when lines carry no cwd.
    """
    if not slug or not slug.startswith("-"):
        return None
    return "/" + slug[1:].replace("-", "/")


def collect_claude(pricing: dict, scope: str | None) -> HarnessResult:
    """Claude Code: ~/.claude/projects/<slug>/*.jsonl with message.usage."""
    root = HOME / ".claude" / "projects"
    res = HarnessResult("claude", str(root), found=root.exists())
    if not root.exists():
        res.note = "no store found"
        return res
    files = sorted(root.rglob("*.jsonl"))
    if not files:
        res.note = "no sessions"
        return res
    for path in files:
        per_file: dict[str, ModelStats] = {}
        cwd_counter: collections.Counter = collections.Counter()
        messages = 0
        # Claude Code logs one line per content block of an API response, each
        # repeating the response's usage. Keep the last usage per message id
        # (streamed lines can carry a growing output count) so a response is
        # counted once.
        usages: dict[str, tuple[str, dict]] = {}
        seen_ids: set[str] = set()
        try:
            with open(path, errors="replace") as f:
                for n, line in enumerate(f):
                    try:
                        obj = json.loads(line)
                    except Exception:
                        continue
                    cwd = obj.get("cwd") or (obj.get("message") or {}).get("cwd")
                    if cwd:
                        cwd_counter[cwd] += 1
                    msg = obj.get("message") or {}
                    role = msg.get("role")
                    if role == "user" or (role == "assistant" and msg.get("id") not in seen_ids):
                        messages += 1
                    if role == "assistant" and msg.get("id"):
                        seen_ids.add(msg["id"])
                    usage = msg.get("usage") or obj.get("usage")
                    model = obj.get("model") or msg.get("model") or "claude"
                    if usage:
                        key = msg.get("id") or obj.get("requestId") or f"line:{n}"
                        usages[key] = (model, usage)
        except Exception:
            continue
        for model, usage in usages.values():
            st = per_file.setdefault(model, ModelStats())
            # Anthropic's input_tokens already excludes cache reads/writes.
            st.input_tokens += int(usage.get("input_tokens", 0) or 0)
            st.output_tokens += int(usage.get("output_tokens", 0) or 0)
            st.cache_read += int(usage.get("cache_read_input_tokens", 0) or 0)
            st.cache_write += int(usage.get("cache_creation_input_tokens", 0) or 0)
        sess_dir = (
            cwd_counter.most_common(1)[0][0]
            if cwd_counter
            else decode_claude_slug(path.parent.name)
        )
        verdict = in_scope(sess_dir, scope)
        if verdict is None:
            res.unattributed += 1
            if scope is not None:
                continue
        elif not verdict:
            res.out_of_scope += 1
            continue
        # attribute file's message count to its dominant model
        dom = max(per_file, key=lambda k: per_file[k].input_tokens + per_file[k].output_tokens) if per_file else "claude"
        for model, st in (per_file or {dom: ModelStats()}).items():
            dst = res.per_model.setdefault(model, ModelStats())
            dst.input_tokens += st.input_tokens
            dst.output_tokens += st.output_tokens
            dst.cache_read += st.cache_read
            dst.cache_write += st.cache_write
            dst.sessions += 1 if model == dom else 0
            dst.messages += messages if model == dom else 0
            est, priced = estimate_cost(model, st.input_tokens, st.output_tokens, st.cache_read, st.cache_write, pricing)
            dst.cost_usd += est
            if not priced:
                dst.unpriced = True
    return res


def collect_gemini(pricing: dict, scope: str | None) -> HarnessResult:
    root = HOME / ".gemini" / "tmp"
    alt = HOME / ".config" / "gemini" / "tmp"
    base = root if root.exists() else alt
    res = HarnessResult("gemini", str(base), found=base.exists())
    if not base.exists():
        res.note = "no store found"
        return res
    files = sorted(base.rglob("*.json"))
    if not files:
        res.note = "no sessions"
        return res
    for path in files:
        try:
            data = json.loads(path.read_text(errors="replace"))
        except Exception:
            continue
        sess_dir = data.get("cwd") or data.get("directory") or data.get("projectRoot") or data.get("root")
        verdict = in_scope(sess_dir, scope)
        if verdict is None:
            res.unattributed += 1
            if scope is not None:
                continue
        elif not verdict:
            res.out_of_scope += 1
            continue
        msgs = data.get("messages") or data.get("history") or []
        model = data.get("model") or "gemini"
        st = res.per_model.setdefault(model, ModelStats())
        st.sessions += 1
        st.messages += len(msgs) if isinstance(msgs, list) else 0
        fi = fo = fr = 0
        for m in msgs if isinstance(msgs, list) else []:
            u = (m or {}).get("usage") or {}
            di = int(u.get("input_tokens", 0) or 0)
            do = int(u.get("output_tokens", 0) or 0)
            dc = int(u.get("cached_content_token_count", 0) or u.get("cache_read", 0) or 0)
            st.input_tokens += di
            st.output_tokens += do
            st.cache_read += dc
            fi += di
            fo += do
            fr += dc
        est, priced = estimate_cost(model, fi, fo, fr, 0, pricing)
        st.cost_usd += est
        if not priced:
            st.unpriced = True
    return res


def collect_dir_based(name: str, paths: list[Path], pricing: dict) -> HarnessResult:
    found = next((p for p in paths if p.exists()), None)
    res = HarnessResult(name, str(found or paths[0]), found=found is not None)
    if found is None:
        res.note = "no store found"
    else:
        res.note = "store present — no machine-readable token log parser yet (counts as 0; extend me)"
    return res


def collect_all(pricing: dict, scope: str | None) -> list[HarnessResult]:
    results = [
        collect_opencode(pricing, scope),
        collect_codex(pricing, scope),
        collect_claude(pricing, scope),
        collect_gemini(pricing, scope),
        collect_dir_based("cursor", [HOME / ".config" / "Cursor", HOME / ".cursor"], pricing),
        collect_dir_based("aider", [HOME / ".aider", Path.cwd() / ".aider.chat.history.md"], pricing),
        collect_dir_based("grok", [HOME / ".grok", HOME / ".xai", HOME / ".config" / "grok"], pricing),
        collect_dir_based("copilot", [HOME / ".config" / "github-copilot", HOME / ".copilot"], pricing),
    ]
    return results


# ---------------------------------------------------------------- output

def fmt_int(n: int) -> str:
    return f"{n:,}"


def fmt_cost(c: float) -> str:
    return f"${c:,.2f}"


def render_text(results: list[HarnessResult], scope: str | None) -> str:
    lines: list[str] = []
    lines.append("Agent usage — sessions, messages, tokens, cost")
    if scope is None:
        lines.append("Scope: all directories (--global)")
    else:
        lines.append(f"Scope: {scope} (incl. subdirectories)")
    lines.append("=" * 72)
    grand = ModelStats()
    for r in results:
        t = r.total
        grand.add(t)
        bits: list[str] = []
        if not (r.found and t.sessions):
            bits.append(r.note if r.note else "no data")
        if scope is not None and (r.out_of_scope or r.unattributed):
            if r.out_of_scope:
                bits.append(f"{r.out_of_scope} outside scope")
            if r.unattributed:
                bits.append(f"{r.unattributed} unattributed (no dir recorded)")
        status = f"  [{'; '.join(bits)}]" if bits else ""
        lines.append(f"\n[{r.harness}] {t.sessions} sessions, {t.messages} messages{status}")
        lines.append(f"  store: {r.store}")
        if not r.per_model:
            continue
        lines.append(f"  {'model':<42} {'sess':>5} {'msg':>6} {'input':>10} {'output':>10} {'cache_rd':>10} {'cache_wr':>10} {'cost':>9}")
        for model in sorted(r.per_model, key=lambda m: r.per_model[m].cost_usd, reverse=True):
            s = r.per_model[model]
            mark = "*" if s.unpriced and s.cost_usd == 0 else ""
            lines.append(
                f"  {model[:42]:<42} {s.sessions:>5} {s.messages:>6} "
                f"{s.input_tokens:>10,} {s.output_tokens:>10,} "
                f"{s.cache_read:>10,} {s.cache_write:>10,} {fmt_cost(s.cost_usd):>9}{mark}"
            )
            if s.reasoning_tokens:
                lines.append(f"    └─ reasoning (subset of output): {s.reasoning_tokens:,} tokens")
    lines.append("\n" + "=" * 72)
    lines.append("TOTAL (all harnesses)")
    lines.append(f"  sessions : {grand.sessions}")
    lines.append(f"  messages : {grand.messages}")
    lines.append(f"  input    : {fmt_int(grand.input_tokens)}")
    lines.append(f"  output   : {fmt_int(grand.output_tokens)}")
    lines.append(f"  cache rd : {fmt_int(grand.cache_read)}")
    lines.append(f"  cache wr : {fmt_int(grand.cache_write)}")
    if grand.reasoning_tokens:
        lines.append(f"  reasoning: {fmt_int(grand.reasoning_tokens)} (subset of output)")
    lines.append(f"  API cost : {fmt_cost(grand.cost_usd)} (estimated)")
    if grand.recorded_cost_usd:
        lines.append(f"  recorded : {fmt_cost(grand.recorded_cost_usd)} (sum of harness-reported cost fields)")
    if any(s.unpriced for r in results for s in r.per_model.values()):
        lines.append("  * unpriced model — pass --pricing pricing.json to set $/1M rates.")
    # per-model rollup across harnesses
    rollup: dict[str, ModelStats] = {}
    for r in results:
        for m, s in r.per_model.items():
            rollup.setdefault(m, ModelStats()).add(s)
    if rollup:
        lines.append("\nPer-model rollup (across harnesses)")
        lines.append(f"  {'model':<42} {'sess':>5} {'msg':>6} {'input':>10} {'output':>10} {'cache_rd':>10} {'cache_wr':>10} {'cost':>9}")
        for model in sorted(rollup, key=lambda m: rollup[m].cost_usd, reverse=True):
            s = rollup[model]
            lines.append(
                f"  {model[:42]:<42} {s.sessions:>5} {s.messages:>6} "
                f"{s.input_tokens:>10,} {s.output_tokens:>10,} "
                f"{s.cache_read:>10,} {s.cache_write:>10,} {fmt_cost(s.cost_usd):>9}"
            )
    return "\n".join(lines) + "\n"


def render_json(results: list[HarnessResult], scope: str | None) -> str:
    out = {"scope": scope, "harnesses": {}, "total": {}, "models": {}}
    grand = ModelStats()
    rollup: dict[str, ModelStats] = {}
    for r in results:
        t = r.total
        grand.add(t)
        for m, s in r.per_model.items():
            rollup.setdefault(m, ModelStats()).add(s)
        out["harnesses"][r.harness] = {
            "store": r.store,
            "found": r.found,
            "note": r.note,
            "sessions_out_of_scope": r.out_of_scope,
            "sessions_unattributed": r.unattributed,
            "sessions": t.sessions,
            "messages": t.messages,
            "input_tokens": t.input_tokens,
            "output_tokens": t.output_tokens,
            "cache_read_tokens": t.cache_read,
            "cache_write_tokens": t.cache_write,
            "reasoning_tokens": t.reasoning_tokens,
            "cost_usd": round(t.cost_usd, 4),
            "recorded_cost_usd": round(t.recorded_cost_usd, 4),
            "by_model": {
                m: {
                    "sessions": s.sessions,
                    "messages": s.messages,
                    "input_tokens": s.input_tokens,
                    "output_tokens": s.output_tokens,
                    "cache_read_tokens": s.cache_read,
                    "cache_write_tokens": s.cache_write,
                    "reasoning_tokens": s.reasoning_tokens,
                    "cost_usd": round(s.cost_usd, 4),
                    "recorded_cost_usd": round(s.recorded_cost_usd, 4),
                    "unpriced": s.unpriced,
                }
                for m, s in r.per_model.items()
            },
        }
    out["total"] = {
        "sessions": grand.sessions,
        "messages": grand.messages,
        "input_tokens": grand.input_tokens,
        "output_tokens": grand.output_tokens,
        "cache_read_tokens": grand.cache_read,
        "cache_write_tokens": grand.cache_write,
        "reasoning_tokens": grand.reasoning_tokens,
        "cost_usd": round(grand.cost_usd, 4),
        "recorded_cost_usd": round(grand.recorded_cost_usd, 4),
    }
    out["models"] = {
        m: {
            "sessions": s.sessions,
            "messages": s.messages,
            "input_tokens": s.input_tokens,
            "output_tokens": s.output_tokens,
            "cache_read_tokens": s.cache_read,
            "cache_write_tokens": s.cache_write,
            "reasoning_tokens": s.reasoning_tokens,
            "cost_usd": round(s.cost_usd, 4),
        }
        for m, s in rollup.items()
    }
    return json.dumps(out, indent=2)


# ---------------------------------------------------------------- main

def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="Sum agent usage across installed coding harnesses.")
    ap.add_argument("--json", action="store_true", help="emit machine-readable JSON")
    ap.add_argument("--harness", default="", help="comma-separated subset, e.g. opencode,codex")
    ap.add_argument("--pricing", default=None, help="path to JSON pricing overrides ($ per 1M tokens)")
    ap.add_argument("--dir", default=None, help="scope to this directory incl. subdirectories (default: current directory)")
    ap.add_argument("--global", "--all", dest="global_scope", action="store_true",
                    help="no directory scoping: count sessions in all directories")
    args = ap.parse_args(argv)
    if args.global_scope and args.dir:
        ap.error("--global cannot be combined with --dir")

    scope = resolve_scope(args.dir, args.global_scope)
    pricing = load_pricing(args.pricing)
    results = collect_all(pricing, scope)
    if args.harness:
        want = {h.strip().lower() for h in args.harness.split(",") if h.strip()}
        results = [r for r in results if r.harness.lower() in want]

    if args.json:
        print(render_json(results, scope))
    else:
        print(render_text(results, scope), end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
