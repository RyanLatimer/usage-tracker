#!/usr/bin/env bun
/**
 * agent-usage TUI — a thin OpenTUI front-end over `usage_tracker.py --json`.
 *
 *   bun tui/index.ts [--dir PATH] [--global] [--pricing FILE] [--harness a,b]
 *   bun tui/index.ts --json | --plain   # old CLI output (also when stdout isn't a TTY)
 *
 * Compiled to a standalone binary with `bun run build` (see package.json);
 * usage_tracker.py and pricing.json are embedded and unpacked to ~/.cache.
 *
 * Follows the terminal's light/dark theme (live); --light / --dark force one.
 *
 * Keys: ↑/↓ pick harness · g toggle global/dir scope · t toggle theme · r refresh · q quit
 */
import {
  BoxRenderable,
  SelectRenderable,
  SelectRenderableEvents,
  StyledText,
  TextRenderable,
  bold,
  createCliRenderer,
  fg,
  type TextChunk,
  type ThemeMode,
} from "@opentui/core"
import { mkdirSync, existsSync, writeFileSync } from "node:fs"
import { homedir } from "node:os"
import { join, resolve } from "node:path"
import scriptSrc from "../usage_tracker.py" with { type: "text" }
import pricingJson from "../pricing.json" with { type: "text" }
const pricingSrc = pricingJson as unknown as string // `type: "text"` makes this a string at runtime

// ---------------------------------------------------------------- data

type Stats = {
  sessions: number
  messages: number
  input_tokens: number
  output_tokens: number
  cache_read_tokens: number
  cache_write_tokens: number
  reasoning_tokens: number
  cost_usd: number
  unpriced?: boolean
}
type Harness = Stats & { store: string; found: boolean; note: string; by_model: Record<string, Stats> }
type Report = { scope: string | null; harnesses: Record<string, Harness>; total: Stats; models: Record<string, Stats> }

// Unpack the embedded python core (keyed by content hash so rebuilds refresh it).
function unpackScript(): string {
  const cache = process.env.XDG_CACHE_HOME || join(homedir(), ".cache")
  const dirPath = join(cache, "agent-usage", Bun.hash(scriptSrc + pricingSrc).toString(16))
  const script = join(dirPath, "usage_tracker.py")
  if (!existsSync(script)) {
    mkdirSync(dirPath, { recursive: true })
    writeFileSync(join(dirPath, "pricing.json"), pricingSrc)
    writeFileSync(script, scriptSrc)
  }
  return script
}
const SCRIPT = unpackScript()

const argv = process.argv.slice(2)

// Old CLI behaviour: --json / --plain / --help, or piped output, go straight to python.
if (argv.some((a) => ["--json", "--plain", "-h", "--help"].includes(a)) || !process.stdout.isTTY) {
  const args = argv.filter((a) => !["--plain", "--light", "--dark"].includes(a))
  const proc = Bun.spawn(["python3", SCRIPT, ...args], { stdio: ["inherit", "inherit", "inherit"] })
  const code = await proc.exited
  if (argv.includes("-h") || argv.includes("--help")) {
    console.log(`
TUI (default when stdout is a terminal):
  --plain         print the text report instead of opening the TUI
  --light/--dark  force a color theme (default: follow the terminal)
  keys: ↑↓ harness · g toggle scope · t theme · r refresh · q quit`)
  }
  process.exit(code)
}

const argVal = (flag: string) => {
  const i = argv.indexOf(flag)
  return i >= 0 ? argv[i + 1] : undefined
}
let globalScope = argv.includes("--global") || argv.includes("--all")
const dir = resolve(argVal("--dir") ?? process.cwd())
const pricing = argVal("--pricing")
const harness = argVal("--harness")

async function load(): Promise<Report> {
  const args = ["python3", SCRIPT, "--json", ...(globalScope ? ["--global"] : ["--dir", dir])]
  if (pricing) args.push("--pricing", pricing)
  if (harness) args.push("--harness", harness)
  const proc = Bun.spawn(args, { stdout: "pipe", stderr: "pipe" })
  const [out, err, code] = await Promise.all([
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
    proc.exited,
  ])
  if (code !== 0) throw new Error(err.trim() || `usage_tracker.py exited ${code}`)
  return JSON.parse(out)
}

// ---------------------------------------------------------------- formatting

const THEMES = {
  dark: {
    text: "#c0caf5",
    dim: "#737aa2",
    accent: "#7aa2f7",
    cost: "#9ece6a",
    warn: "#e0af68",
    error: "#f7768e",
    border: "#3b4261",
    bar: "#7aa2f7",
    selected: "#283457",
  },
  light: {
    text: "#1f2335",
    dim: "#6172b0",
    accent: "#2e59c9",
    cost: "#2f7a33",
    warn: "#8c5a00",
    error: "#c4314b",
    border: "#a8aecb",
    bar: "#2e59c9",
    selected: "#d6def5",
  },
} satisfies Record<ThemeMode, Record<string, string>>
type Theme = (typeof THEMES)["dark"]
let C: Theme = THEMES.dark

const compact = (n: number) => {
  const abs = Math.abs(n)
  if (abs >= 1e9) return (n / 1e9).toFixed(2) + "B"
  if (abs >= 1e6) return (n / 1e6).toFixed(1) + "M"
  if (abs >= 1e3) return (n / 1e3).toFixed(1) + "K"
  return String(n)
}
const money = (n: number) => "$" + n.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })
const pad = (s: string, w: number) => (s.length > w ? s.slice(0, w - 1) + "…" : s.padEnd(w))
const lpad = (s: string, w: number) => s.padStart(w)
const styled = (...chunks: TextChunk[]) => new StyledText(chunks)
const text = (s: string, color = C.text) => fg(color)(s)

function bar(frac: number, width: number): string {
  const cells = Math.max(0, frac) * width
  const full = Math.floor(cells)
  const part = " ▏▎▍▌▋▊▉"[Math.round((cells - full) * 8)] ?? ""
  return ("█".repeat(full) + (full < width ? part : "")).padEnd(width)
}

// ---------------------------------------------------------------- layout

const renderer = await createCliRenderer({ exitOnCtrlC: true })

const root = new BoxRenderable(renderer, { flexDirection: "column", width: "100%", height: "100%", padding: 1, gap: 1 })
renderer.root.add(root)

const header = new TextRenderable(renderer, { content: "" })
root.add(header)

// stat tiles
const boxes: BoxRenderable[] = []
const tilesRow = new BoxRenderable(renderer, { flexDirection: "row", gap: 1, height: 3, flexShrink: 0 })
root.add(tilesRow)
const tile = (title: string, grow = 1) => {
  const box = new BoxRenderable(renderer, {
    border: true, borderStyle: "rounded", overflow: "hidden",
    title: ` ${title} `, flexGrow: grow, flexBasis: 0, minWidth: title.length + 4, paddingLeft: 1,
  })
  const value = new TextRenderable(renderer, { content: "…", wrapMode: "none" })
  box.add(value)
  tilesRow.add(box)
  boxes.push(box)
  return value
}
const tCost = tile("API cost")
const tSessions = tile("Sessions")
const tMessages = tile("Messages")
const tTokens = tile("Tokens", 3)

// body: harness list + model table
const body = new BoxRenderable(renderer, { flexDirection: "row", gap: 1, flexGrow: 1 })
root.add(body)

const listBox = new BoxRenderable(renderer, {
  border: true, borderStyle: "rounded",
  title: " Harness ", width: 30, flexShrink: 0,
})
const list = new SelectRenderable(renderer, {
  flexGrow: 1,
  options: [],
  backgroundColor: "transparent",
  focusedBackgroundColor: "transparent",
  wrapSelection: true,
})
listBox.add(list)
body.add(listBox)

const tableBox = new BoxRenderable(renderer, {
  border: true, borderStyle: "rounded",
  title: " Models ", flexGrow: 1, paddingLeft: 1, paddingRight: 1, overflow: "hidden",
})
const table = new TextRenderable(renderer, { content: "", wrapMode: "none" })
tableBox.add(table)
body.add(tableBox)
boxes.push(listBox, tableBox)

const footer = new TextRenderable(renderer, { content: "" })
root.add(footer)

// ---------------------------------------------------------------- render

let report: Report | null = null
let status: { msg: string; kind: "warn" | "error" } | null = null

function renderFooter() {
  const keys = [["↑↓", "harness"], ["g", "toggle scope"], ["t", "theme"], ["r", "refresh"], ["q", "quit"]]
  footer.content = styled(...keys.flatMap(([k, label]) => [text(k!, C.accent), text(` ${label}   `, C.dim)]))
}

function renderHeader() {
  const scope = globalScope ? "all directories" : `${dir}  (incl. subdirs)`
  header.content = styled(
    bold(text("agent-usage", C.accent)),
    text("  ·  ", C.dim),
    text(scope, C.dim),
    ...(status ? [text("   " + status.msg, C[status.kind])] : []),
  )
}

function renderTiles(r: Report) {
  const t = r.total
  tCost.content = styled(bold(text(money(t.cost_usd), C.cost)))
  tSessions.content = styled(bold(text(t.sessions.toLocaleString())))
  tMessages.content = styled(bold(text(t.messages.toLocaleString())))
  tTokens.content = styled(
    bold(text(compact(t.input_tokens + t.output_tokens + t.cache_read_tokens + t.cache_write_tokens))),
    text(`  in ${compact(t.input_tokens)} · out ${compact(t.output_tokens)} · cache ${compact(t.cache_read_tokens + t.cache_write_tokens)}`, C.dim),
  )
}

function renderList(r: Report) {
  const active = Object.entries(r.harnesses).filter(([, h]) => h.sessions > 0)
  const prev = list.getSelectedOption()?.value
  const options = [
    { name: "All harnesses", description: `${money(r.total.cost_usd)} · ${r.total.sessions} sess`, value: "*" },
    ...active
      .sort(([, a], [, b]) => b.cost_usd - a.cost_usd)
      .map(([name, h]) => ({ name, description: `${money(h.cost_usd)} · ${h.sessions} sess`, value: name })),
  ]
  list.options = options
  const idx = options.findIndex((o) => o.value === prev)
  list.setSelectedIndex(Math.max(0, idx))

  const idle = Object.entries(r.harnesses).filter(([, h]) => h.found && h.sessions === 0).map(([n]) => n)
  listBox.bottomTitle = idle.length ? ` idle: ${idle.join(", ")} ` : undefined
}

function renderTable() {
  if (!report) return
  const key = list.getSelectedOption()?.value ?? "*"
  const h = key === "*" ? null : report.harnesses[key]
  const models = h ? h.by_model : report.models
  tableBox.title = h ? ` ${key} — ${h.store} ` : " All models "

  const rows = Object.entries(models).sort(([, a], [, b]) => b.cost_usd - a.cost_usd)
  if (!rows.length) {
    table.content = styled(text(h?.note || "no sessions in scope", C.dim))
    return
  }
  const maxCost = Math.max(...rows.map(([, s]) => s.cost_usd), 1e-9)
  // numeric columns in display order; [3] = drop priority (highest dropped first)
  // until they fit the pane (terminal - root padding 2 - list 30 - gap 1 - border 2 - padding 2)
  let cols: [string, number, (s: Stats) => string, number][] = [
    ["sess", 7, (s) => String(s.sessions), 0],
    ["msgs", 7, (s) => String(s.messages), 4],
    ["input", 9, (s) => compact(s.input_tokens), 2],
    ["output", 9, (s) => compact(s.output_tokens), 1],
    ["cache rd", 10, (s) => compact(s.cache_read_tokens), 3],
    ["cache wr", 10, (s) => compact(s.cache_write_tokens), 5],
  ]
  const avail = renderer.width - 37
  const COST = 10
  const width = () => cols.reduce((n, [, w]) => n + w, 0) + COST
  while (cols.length > 1 && avail - width() < 16) {
    const drop = Math.max(...cols.map((c) => c[3]))
    cols = cols.filter((c) => c[3] !== drop)
  }
  const barW = avail - width() >= 38 ? Math.min(20, avail - width() - 24) : 0
  const modelW = Math.max(12, avail - width() - (barW ? barW + 2 : 0))

  const chunks: TextChunk[] = [
    text(pad("model", modelW) + cols.map(([h, w]) => lpad(h, w)).join("") + lpad("cost", COST) + "\n", C.dim),
  ]
  for (const [model, s] of rows) {
    const unpriced = s.unpriced && s.cost_usd === 0
    chunks.push(
      text(pad(model, modelW)),
      text(cols.map(([, w, f]) => lpad(f(s), w)).join(""), C.dim),
      unpriced ? text(lpad("unpriced", COST), C.warn) : bold(text(lpad(money(s.cost_usd), COST), C.cost)),
      text((barW ? "  " + bar(s.cost_usd / maxCost, barW) : "") + "\n", C.bar),
    )
  }
  if (h?.note) chunks.push(text("\n" + h.note, C.dim))
  table.content = styled(...chunks)
}

let loading = false
async function refresh() {
  if (loading) return
  loading = true
  status = { msg: "loading…", kind: "warn" }
  renderHeader()
  try {
    report = await load()
    status = null
    renderHeader()
    renderTiles(report)
    renderList(report)
    renderTable()
  } catch (e) {
    status = { msg: `error: ${(e as Error).message}`, kind: "error" }
    renderHeader()
  } finally {
    loading = false
  }
}

function applyTheme(mode: ThemeMode) {
  C = THEMES[mode]
  for (const b of boxes) {
    b.borderColor = C.border
    b.titleColor = C.dim
  }
  list.textColor = C.text
  list.focusedTextColor = C.text
  list.descriptionColor = C.dim
  list.selectedBackgroundColor = C.selected
  list.selectedTextColor = C.accent
  list.selectedDescriptionColor = C.cost
  renderHeader()
  renderFooter()
  if (report) {
    renderTiles(report)
    renderTable()
  }
}

// --light/--dark pin the theme; otherwise follow the terminal (and its live changes)
const forced: ThemeMode | null = argv.includes("--light") ? "light" : argv.includes("--dark") ? "dark" : null
let theme: ThemeMode = forced ?? (await renderer.waitForThemeMode(300)) ?? "dark"
applyTheme(theme)
renderer.on("theme_mode", (mode: ThemeMode) => {
  if (forced) return
  theme = mode
  applyTheme(mode)
})

list.on(SelectRenderableEvents.SELECTION_CHANGED, renderTable)
renderer.on("resize", renderTable)
list.focus()

renderer.keyInput.on("keypress", (key) => {
  if (key.name === "q" || key.name === "escape") {
    renderer.destroy()
    process.exit(0)
  } else if (key.name === "r") {
    refresh()
  } else if (key.name === "t") {
    theme = theme === "dark" ? "light" : "dark"
    applyTheme(theme)
  } else if (key.name === "g") {
    globalScope = !globalScope
    refresh()
  }
})

refresh()
