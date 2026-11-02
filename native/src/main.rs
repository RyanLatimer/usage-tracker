use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

use rune::{col, el, row, text, AppSpec, Cx, El};
use rune::{Color, ScrollSpec, TextStyle};
use rune_kit::{
    Badge, Button, ButtonVariant, ControlSize, Divider, Kit, List, ListItem, SegmentedControl,
};

// ---------------------------------------------------------------- data

const SCRIPT: &str = include_str!("../../usage_tracker.py");
const PRICING: &str = include_str!("../../pricing.json");

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct Stats {
    #[serde(default)]
    sessions: u64,
    #[serde(default)]
    messages: u64,
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_read_tokens: u64,
    #[serde(default)]
    cache_write_tokens: u64,
    #[serde(default)]
    cost_usd: f64,
    #[serde(default)]
    unpriced: bool,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct Harness {
    #[serde(default)]
    found: bool,
    #[serde(default)]
    sessions: u64,
    #[serde(default)]
    messages: u64,
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_read_tokens: u64,
    #[serde(default)]
    cache_write_tokens: u64,
    #[serde(default)]
    cost_usd: f64,
    #[serde(default)]
    by_model: HashMap<String, Stats>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct Report {
    #[serde(default)]
    daily: HashMap<String, std::collections::BTreeMap<String, HashMap<String, Stats>>>,
    #[serde(default)]
    chart_dates: Vec<String>,
    #[serde(default)]
    harnesses: HashMap<String, Harness>,
    #[serde(default)]
    total: Stats,
    #[serde(default)]
    models: HashMap<String, Stats>,
}

fn cache_script() -> PathBuf {
    let base = std::env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dirs_home().join(".cache"));
    let dir = base
        .join("agent-usage-rune")
        .join(format!("{:x}", cache_hash()));
    let script = dir.join("usage_tracker.py");
    if !script.exists() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("pricing.json"), PRICING);
        let _ = std::fs::write(&script, SCRIPT);
    }
    script
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn cache_hash() -> u64 {
    use std::hash::Hash;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    SCRIPT.hash(&mut h);
    PRICING.hash(&mut h);
    std::hash::Hasher::finish(&h)
}

fn load_report(global: bool) -> Result<Report, String> {
    let script = cache_script();
    let mut args = vec![script.to_string_lossy().to_string(), "--json".to_string()];
    if global {
        args.push("--global".to_string());
    } else {
        args.push("--dir".to_string());
        args.push(
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .to_string_lossy()
                .to_string(),
        );
    }
    let out = Command::new("python3")
        .args(&args)
        .output()
        .map_err(|e| format!("python3 not found ({e}): brew install python"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() {
            format!("usage_tracker.py exited {}", out.status)
        } else {
            err
        });
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("bad JSON: {e}"))
}

impl Stats {
    fn add(&mut self, s: &Self) {
        self.sessions += s.sessions;
        self.messages += s.messages;
        self.input_tokens += s.input_tokens;
        self.output_tokens += s.output_tokens;
        self.cache_read_tokens += s.cache_read_tokens;
        self.cache_write_tokens += s.cache_write_tokens;
        self.cost_usd += s.cost_usd;
        self.unpriced |= s.unpriced;
    }
}
fn poll(s: &mut State, _: f64, _: &mut rune::Tx) -> bool {
    let result = s.pending.as_ref().and_then(|rx| rx.try_recv().ok());
    if let Some(result) = result {
        s.pending = None;
        match result {
            Ok(report) => {
                s.all_report = report;
                s.apply_period();
                s.error = None;
            }
            Err(e) => s.error = Some(e),
        }
        return true;
    }
    false
}
// ---------------------------------------------------------------- state

struct State {
    appearance: u8, // 0 auto, 1 light, 2 dark
    global_scope: bool,
    selected: String,
    report: Report,
    error: Option<String>,
    all_report: Report,
    range_days: usize,
    pending: Option<std::sync::mpsc::Receiver<Result<Report, String>>>,
    waker: Option<rune::Waker>,
}

impl State {
    fn init() -> Self {
        let global_scope = true;
        let (report, error) = if rune::is_headless() {
            match load_report(global_scope) {
                Ok(r) => (r, None),
                Err(e) => (Report::default(), Some(e)),
            }
        } else {
            (Report::default(), None)
        };
        State {
            appearance: 0,
            global_scope,
            selected: "*".into(),
            all_report: report.clone(),
            report,
            error,
            range_days: 0,
            pending: None,
            waker: None,
        }
    }
    fn reload(&mut self) {
        let global = self.global_scope;
        let (tx, rx) = std::sync::mpsc::channel();
        self.pending = Some(rx);
        let wake = self.waker.clone();
        std::thread::spawn(move || {
            let _ = tx.send(load_report(global));
            if let Some(wake) = wake {
                wake();
            }
        });
    }
    fn apply_period(&mut self) {
        self.report = self.all_report.clone();
        if self.range_days == 0 {
            return;
        }
        self.report.models.clear();
        self.report.total = Stats::default();
        let dates: Vec<_> = self
            .all_report
            .chart_dates
            .iter()
            .rev()
            .take(self.range_days)
            .collect();
        for (name, h) in &mut self.report.harnesses {
            h.by_model.clear();
            if let Some(days) = self.all_report.daily.get(name) {
                for date in &dates {
                    if let Some(bucket) = days.get(*date) {
                        for (model, st) in bucket {
                            h.by_model.entry(model.clone()).or_default().add(st);
                        }
                    }
                }
            }
            let mut total = Stats::default();
            for (model, st) in &h.by_model {
                total.add(st);
                self.report.models.entry(model.clone()).or_default().add(st);
            }
            h.sessions = total.sessions;
            h.messages = total.messages;
            h.input_tokens = total.input_tokens;
            h.output_tokens = total.output_tokens;
            h.cache_read_tokens = total.cache_read_tokens;
            h.cache_write_tokens = total.cache_write_tokens;
            h.cost_usd = total.cost_usd;
            self.report.total.add(&total);
        }
    }

    fn dark(&self, system_dark: bool) -> bool {
        match self.appearance {
            1 => false,
            2 => true,
            _ => system_dark,
        }
    }
}

// ---------------------------------------------------------------- format

fn compact(n: u64) -> String {
    let f = n as f64;
    if f >= 1e9 {
        format!("{:.2}B", f / 1e9)
    } else if f >= 1e6 {
        format!("{:.1}M", f / 1e6)
    } else if f >= 1e3 {
        format!("{:.1}K", f / 1e3)
    } else {
        format!("{n}")
    }
}

fn money(n: f64) -> String {
    format!("${:.2}", n)
}

// ---------------------------------------------------------------- design

fn ink(hex: &str) -> Color {
    rune::color::hex(hex).expect("design color")
}

#[derive(Clone, Copy)]
struct Theme {
    kit: &'static Kit,
    page: Color,
    sidebar: Color,
    panel: Color,
    line: Color,
    text: Color,
    muted: Color,
    faint: Color,
    accent: Color,
}
impl Theme {
    fn new(dark: bool) -> Self {
        let k = rune_theme_opencode::kit(dark);
        Self {
            kit: k,
            page: k.c.page,
            sidebar: k.c.bg_base,
            panel: k.c.layer_01,
            line: k.c.border_muted,
            text: k.c.text_base,
            muted: k.c.text_muted,
            faint: k.c.text_faint,
            accent: k.c.text_accent,
        }
    }
}
fn label(key: &str, value: impl Into<String>, size: f32, weight: f32, color: Color) -> El {
    text(value.into())
        .key(key)
        .font(TextStyle::new(size, weight).features("'tnum' 1"))
        .color(color)
}
fn rule(key: &str, t: Theme) -> El {
    Divider::horizontal().build(t.kit).key(key)
}
fn dot(key: &str, color: Color) -> El {
    el().key(key).size(7.0, 7.0).radius(4.0).fill(color)
}
fn source_color(name: &str) -> Color {
    ink(match name {
        "opencode" => "#A8E6BE",
        "claude" => "#E6B294",
        "codex" => "#9CBFEB",
        "cursor" => "#C6B2EB",
        "copilot" => "#E2CB87",
        _ => "#8CBDB4",
    })
}
fn source_name(name: &str) -> String {
    match name {
        "opencode" => "OpenCode",
        "claude" => "Claude Code",
        "codex" => "Codex",
        "cursor" => "Cursor",
        "copilot" => "GitHub Copilot",
        "grok" => "Grok",
        _ => name,
    }
    .to_string()
}
fn selected_stats(s: &State) -> Stats {
    if s.selected == "*" {
        return s.report.total.clone();
    }
    s.report
        .harnesses
        .get(&s.selected)
        .map(|h| Stats {
            sessions: h.sessions,
            messages: h.messages,
            input_tokens: h.input_tokens,
            output_tokens: h.output_tokens,
            cache_read_tokens: h.cache_read_tokens,
            cache_write_tokens: h.cache_write_tokens,
            cost_usd: h.cost_usd,
            ..Stats::default()
        })
        .unwrap_or_default()
}
fn models(s: &State) -> Vec<(&String, &Stats)> {
    let mut rows: Vec<_> = if s.selected == "*" {
        s.report.models.iter().collect()
    } else {
        s.report
            .harnesses
            .get(&s.selected)
            .map(|h| h.by_model.iter().collect())
            .unwrap_or_default()
    };
    rows.sort_by(|a, b| b.1.cost_usd.total_cmp(&a.1.cost_usd).then(a.0.cmp(b.0)));
    rows
}
fn view(s: &State, cx: &mut Cx<State>) -> El {
    let t = Theme::new(s.dark(cx.system_dark()));
    let chrome = row()
        .key("chrome")
        .w_full()
        .h(38.0)
        .px(18.0)
        .center_y()
        .window_drag()
        .child(el().key("traffic-space").w(cx.titlebar_inset().max(78.0)));
    let content = col()
        .key("content")
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .padding(24.0)
        .gap(20.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(header(s, t))
        .child(period_controls(s, t))
        .child(summary(s, t))
        .child(daily_chart(s, t))
        .child(token_totals(s, t))
        .child(allocation(s, t))
        .child(model_table(s, t));
    let body = col()
        .key("window")
        .w_full()
        .h_full()
        .fill(t.page)
        .child(chrome)
        .child(
            row()
                .key("workspace")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .child(sidebar(s, t))
                .child(content),
        );
    rune_kit::root(body, Vec::<El>::new())
}
fn action(key: &str, title: &str, t: Theme, f: impl Fn(&mut State) + 'static) -> El {
    Button::new(title)
        .control_size(ControlSize::Large)
        .on_press(f)
        .build(t.kit)
        .key(key)
}
fn header(s: &State, t: Theme) -> El {
    let title = if s.selected == "*" {
        "Usage".into()
    } else {
        source_name(&s.selected)
    };
    row()
        .key("header")
        .w_full()
        .center_y()
        .justify_between()
        .gap(12.0)
        .child(
            col()
                .key("heading")
                .gap(6.0)
                .grow(1.0)
                .min_w(0.0)
                .child(label("title", title, 22.0, 600.0, t.text)),
        )
        .child(
            row()
                .key("actions")
                .gap(8.0)
                .child(action(
                    "scope-btn",
                    if s.global_scope {
                        "All directories"
                    } else {
                        "Current directory"
                    },
                    t,
                    |s| {
                        s.global_scope = !s.global_scope;
                        s.reload();
                    },
                ))
                .child(action(
                    "refresh-btn",
                    if s.pending.is_some() {
                        "Refreshing…"
                    } else {
                        "Refresh"
                    },
                    t,
                    |s| s.reload(),
                )),
        )
}
fn sidebar(s: &State, t: Theme) -> El {
    let mut choices = vec![("*".to_string(), "All sources".to_string())];
    let mut names: Vec<_> = s
        .report
        .harnesses
        .iter()
        .filter(|(_, h)| h.found || h.sessions > 0)
        .collect();
    names.sort_by(|a, b| a.0.cmp(b.0));
    choices.extend(
        names
            .into_iter()
            .map(|(name, _)| (name.clone(), source_name(name))),
    );
    let active = choices.iter().position(|(key, _)| *key == s.selected);
    let mut nav = List::new().active(active);
    for (key, title) in choices {
        nav = nav.item(
            ListItem::new(title)
                .selected(key == s.selected)
                .on_select(move |s: &mut State, _| s.selected = key.clone()),
        );
    }
    let theme = match s.appearance {
        1 => "Light",
        2 => "Dark",
        _ => "System",
    };
    col()
        .key("sidebar")
        .w(184.0)
        .h_full()
        .fill(t.sidebar)
        .padding(14.0)
        .gap(16.0)
        .child(
            text("Sources")
                .key("sources-title")
                .font(t.kit.ty.small_medium)
                .color(t.muted)
                .px(8.0)
                .py(8.0),
        )
        .child(
            col()
                .key("nav-scroll")
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(nav.build(t.kit).key("source-list")),
        )
        .child(
            Button::new(theme)
                .variant(ButtonVariant::GhostMuted)
                .help("Appearance · T")
                .on_press(|s: &mut State| s.appearance = (s.appearance + 1) % 3)
                .build(t.kit)
                .key("theme-btn"),
        )
}
fn summary(s: &State, t: Theme) -> El {
    let st = selected_stats(s);
    let tokens = st.input_tokens + st.output_tokens + st.cache_read_tokens + st.cache_write_tokens;
    let mut hero = col()
        .key("spend")
        .grow(1.15)
        .min_w(0.0)
        .padding(18.0)
        .gap(8.0)
        .radius(t.kit.radius.md)
        .fill(t.panel)
        .border(1.0, t.line)
        .help("Estimated API cost; includes cached tokens")
        .child(label(
            "spend-label",
            "API cost (est.)",
            10.0,
            600.0,
            t.accent,
        ))
        .child(label(
            "spend-value",
            money(st.cost_usd),
            42.0,
            500.0,
            t.text,
        ));
    if let Some(e) = &s.error {
        hero = hero.child(label("error", e, 11.0, 400.0, ink("#D59A7A")).max_lines(3));
    }
    let avg = if st.sessions > 0 {
        st.cost_usd / st.sessions as f64
    } else {
        0.0
    };
    row().key("summary").w_full().gap(14.0).child(hero).child(
        col()
            .key("metrics")
            .grow(1.0)
            .min_w(0.0)
            .padding(18.0)
            .gap(12.0)
            .radius(t.kit.radius.md)
            .fill(t.panel)
            .border(1.0, t.line)
            .child(
                row()
                    .key("metric-top")
                    .gap(20.0)
                    .child(metric("sessions", "Sessions", compact(st.sessions), t))
                    .child(metric("messages", "Messages", compact(st.messages), t)),
            )
            .child(rule("metric-rule", t))
            .child(
                row()
                    .key("metric-bottom")
                    .gap(20.0)
                    .child(metric("tokens", "Tokens", compact(tokens), t))
                    .child(metric("average", "Cost / session", money(avg), t)),
            ),
    )
}
fn metric(key: &str, title: &str, value: String, t: Theme) -> El {
    col()
        .key(key)
        .grow(1.0)
        .min_w(0.0)
        .gap(5.0)
        .child(label(&format!("{key}-label"), title, 10.0, 400.0, t.muted))
        .child(label(&format!("{key}-value"), value, 21.0, 500.0, t.text))
}
fn allocation(s: &State, t: Theme) -> El {
    let mut entries: Vec<(String, f64, Color)> = if s.selected == "*" {
        s.report
            .harnesses
            .iter()
            .filter(|(_, h)| h.cost_usd > 0.0)
            .map(|(n, h)| (source_name(n), h.cost_usd, source_color(n)))
            .collect()
    } else {
        models(s)
            .iter()
            .filter(|(_, st)| st.cost_usd > 0.0)
            .enumerate()
            .map(|(i, (n, st))| {
                (
                    n.rsplit('/').next().unwrap_or(n).to_string(),
                    st.cost_usd,
                    chart_color(i),
                )
            })
            .collect()
    };
    entries.sort_by(|a, b| b.1.total_cmp(&a.1));
    let total = entries.iter().map(|e| e.1).sum::<f64>();
    let mut bar = row()
        .key("allocation-bar")
        .w_full()
        .h(10.0)
        .gap(3.0)
        .clip_rounded(5.0);
    let mut legend = row().key("allocation-legend").gap(22.0).w_full();
    for (i, (name, cost, color)) in entries.iter().enumerate() {
        bar = bar.child(
            el().key(format!("segment-{i}"))
                .h_full()
                .grow((*cost / total) as f32)
                .fill(*color),
        );
        if i < 3 {
            legend = legend.child(
                row()
                    .key(format!("legend-{i}"))
                    .center_y()
                    .gap(7.0)
                    .child(dot(&format!("legend-dot-{i}"), *color))
                    .child(label(
                        &format!("legend-name-{i}"),
                        format!("{}  {:.0}%", name, 100.0 * cost / total),
                        10.0,
                        400.0,
                        t.muted,
                    )),
            );
        }
    }
    if entries.is_empty() {
        bar = bar.child(el().key("allocation-empty").w_full().h_full().fill(t.line));
        legend = legend.child(label(
            "no-spend",
            "No recorded cost in this scope",
            11.0,
            400.0,
            t.muted,
        ));
    } else if entries.len() > 3 {
        legend = legend.child(label(
            "allocation-others",
            format!("+ {} more", entries.len() - 3),
            10.0,
            400.0,
            t.faint,
        ));
    }
    col()
        .key("allocation")
        .gap(13.0)
        .child(
            row()
                .key("allocation-heading")
                .w_full()
                .justify_between()
                .child(label(
                    "allocation-title",
                    if s.selected == "*" {
                        "Spend by source"
                    } else {
                        "Spend by model"
                    },
                    12.0,
                    500.0,
                    t.text,
                )),
        )
        .child(bar)
        .child(legend)
}
fn chart_color(i: usize) -> Color {
    ink(["#A8E6BE", "#9CBFEB", "#E6B294", "#C6B2EB", "#E2CB87"][i % 5])
}
fn model_table(s: &State, t: Theme) -> El {
    let rows = models(s);
    let total = selected_stats(s).cost_usd;
    let mut list = col().key("model-rows").w_full();
    for (i, (name, st)) in rows.iter().enumerate() {
        let id = format!("model-{name}");
        let (provider, model) = name.rsplit_once('/').unwrap_or(("", name.as_str()));
        let share = if total > 0.0 {
            (st.cost_usd / total).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let mut bar = row()
            .key(format!("{id}-bar"))
            .w(56.0)
            .h(4.0)
            .radius(2.0)
            .fill(t.line);
        if share > 0.0 {
            bar = bar.child(
                el().key(format!("{id}-bar-fill"))
                    .w((56.0 * share) as f32)
                    .h(4.0)
                    .radius(2.0)
                    .fill(t.accent),
            );
        }
        list = list.child(rule(&format!("{id}-rule"), t)).child(
            row()
                .key(&id)
                .w_full()
                .h(48.0)
                .px(16.0)
                .gap(12.0)
                .center_y()
                .hover(t.sidebar)
                .child(
                    label(
                        &format!("{id}-rank"),
                        format!("{:02}", i + 1),
                        10.0,
                        400.0,
                        t.faint,
                    )
                    .w(22.0),
                )
                .child(
                    col()
                        .key(format!("{id}-identity"))
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(3.0)
                        .child(
                            label(&format!("{id}-name"), model, 12.0, 500.0, t.text).max_lines(1),
                        )
                        .children(if provider.is_empty() {
                            Vec::new()
                        } else {
                            vec![
                                label(&format!("{id}-provider"), provider, 10.0, 400.0, t.faint)
                                    .max_lines(1),
                            ]
                        }),
                )
                .child(
                    label(
                        &format!("{id}-sessions"),
                        compact(st.sessions),
                        11.0,
                        400.0,
                        t.muted,
                    )
                    .w(58.0),
                )
                .child(
                    label(
                        &format!("{id}-tokens"),
                        compact(
                            st.input_tokens
                                + st.output_tokens
                                + st.cache_read_tokens
                                + st.cache_write_tokens,
                        ),
                        11.0,
                        400.0,
                        t.muted,
                    )
                    .w(65.0),
                )
                .child(
                    col()
                        .key(format!("{id}-cost-col"))
                        .w(76.0)
                        .gap(4.0)
                        .child(label(
                            &format!("{id}-cost"),
                            if st.unpriced && st.cost_usd == 0.0 {
                                "Unpriced".into()
                            } else {
                                money(st.cost_usd)
                            },
                            12.0,
                            500.0,
                            t.text,
                        ))
                        .child(bar),
                ),
        );
    }
    if rows.is_empty() {
        list = list.child(col().key("empty-state").padding(30.0).gap(8.0).child(label(
            "empty-title",
            "No usage",
            16.0,
            500.0,
            t.text,
        )));
    }
    col()
        .key("models")
        .w_full()
        .gap(14.0)
        .child(
            row()
                .key("models-heading")
                .w_full()
                .justify_between()
                .center_y()
                .child(
                    row()
                        .key("models-title-row")
                        .gap(9.0)
                        .center_y()
                        .child(label("models-title", "Models", 14.0, 600.0, t.text))
                        .child(
                            Badge::new(rows.len().to_string())
                                .build(t.kit)
                                .key("models-count"),
                        ),
                ),
        )
        .child(
            col()
                .key("model-table")
                .w_full()
                .radius(10.0)
                .clip_rounded(10.0)
                .fill(t.panel)
                .border(1.0, t.line)
                .child(
                    row()
                        .key("column-headings")
                        .w_full()
                        .h(33.0)
                        .px(16.0)
                        .gap(12.0)
                        .center_y()
                        .child(label("column-model", "MODEL", 9.0, 500.0, t.faint).grow(1.0))
                        .child(label("column-sessions", "SESSIONS", 9.0, 500.0, t.faint).w(58.0))
                        .child(label("column-tokens", "TOKENS", 9.0, 500.0, t.faint).w(65.0))
                        .child(label("column-cost", "COST", 9.0, 500.0, t.faint).w(76.0)),
                )
                .child(list),
        )
}

fn period_controls(s: &State, t: Theme) -> El {
    let selected = [0, 7, 30, 90].iter().position(|days| *days == s.range_days);
    row().key("period").w_full().justify_end().child(
        SegmentedControl::new(&["All time", "7 days", "30 days", "90 days"], selected)
            .fit()
            .on_pick(|s: &mut State, index, _| {
                s.range_days = [0, 7, 30, 90][index];
                s.apply_period();
            })
            .build(t.kit)
            .key("period-options"),
    )
}
fn daily_chart(s: &State, t: Theme) -> El {
    let days = if s.range_days == 0 { 30 } else { s.range_days };
    let dates: Vec<_> = s
        .all_report
        .chart_dates
        .iter()
        .rev()
        .take(days)
        .rev()
        .collect();
    let mut values = Vec::new();
    for date in &dates {
        let cost: f64 = s
            .all_report
            .daily
            .iter()
            .filter(|(source, _)| s.selected == "*" || s.selected == **source)
            .filter_map(|(_, d)| d.get(*date))
            .flat_map(|bucket| bucket.values())
            .map(|st| st.cost_usd)
            .sum();
        values.push(cost);
    }
    let peak = values.iter().copied().fold(0.0_f64, f64::max);
    let mut bars = row().key("daily-bars").w_full().h(104.0).gap(3.0);
    for (i, cost) in values.iter().enumerate() {
        bars = bars.child(
            col()
                .key(format!("day-{i}"))
                .help(format!("{} · {}", dates[i], money(*cost)))
                .grow(1.0)
                .min_w(0.0)
                .h_full()
                .justify_end()
                .child(
                    el().key(format!("day-bar-{i}"))
                        .w_full()
                        .h(if peak > 0.0 {
                            (cost / peak * 100.0).max(2.0) as f32
                        } else {
                            2.0
                        })
                        .radius(2.0)
                        .fill(if *cost > 0.0 { t.accent } else { t.line }),
                ),
        );
    }
    col()
        .key("daily-chart")
        .help("UTC · cost grouped by session start date. Undated sessions appear in All time only. All time chart shows the last 30 days.")
        .w_full()
        .gap(12.0)
        .child(
            row()
                .key("daily-header")
                .w_full()
                .justify_between()
                .child(label("daily-title", "Daily cost", 13.0, 500.0, t.text))
                .child(label(
                    "daily-peak",
                    format!("Peak {} / day", money(peak)),
                    10.0,
                    400.0,
                    t.muted,
                )),
        )
        .child(bars)
        .child(rule("daily-baseline", t))
        .child(
            row()
                .key("daily-axis")
                .w_full()
                .justify_between()
                .child(label(
                    "daily-start",
                    dates.first().map(|d| d.as_str()).unwrap_or(""),
                    10.0,
                    400.0,
                    t.faint,
                ))
                .child(label(
                    "daily-end",
                    dates.last().map(|d| d.as_str()).unwrap_or(""),
                    10.0,
                    400.0,
                    t.faint,
                )),
        )
}
fn token_totals(s: &State, t: Theme) -> El {
    let st = selected_stats(s);
    row()
        .key("token-totals")
        .w_full()
        .py(16.0)
        .gap(18.0)
        .child(metric(
            "fresh-tokens",
            "Uncached input",
            compact(st.input_tokens),
            t,
        ))
        .child(metric(
            "cache-read",
            "Cache read",
            compact(st.cache_read_tokens),
            t,
        ))
        .child(metric(
            "cache-write",
            "Cache write",
            compact(st.cache_write_tokens),
            t,
        ))
        .child(metric(
            "output-tokens",
            "Output",
            compact(st.output_tokens),
            t,
        ))
}

fn on_key(s: &mut State, ev: &rune::KeyEvent, _tx: &mut rune::Tx) -> bool {
    use rune::Key;
    if ev.mods != rune::Mods::default() {
        return false;
    }
    match &ev.key {
        Key::Text(t) if t == "g" => {
            s.global_scope = !s.global_scope;
            s.reload();
            true
        }
        Key::Text(t) if t == "r" => {
            s.reload();
            true
        }
        Key::Text(t) if t == "t" => {
            s.appearance = (s.appearance + 1) % 3;
            true
        }
        _ => false,
    }
}

fn main() {
    rune::run(AppSpec {
        scenes: &[
            ("default", |_: &mut State| {}),
            ("demo", |s: &mut State| {
                s.all_report = serde_json::from_str(include_str!("../fixtures/usage.json"))
                    .expect("demo fixture");
                s.report = s.all_report.clone();
                s.error = None;
            }),
            ("demo-light", |s: &mut State| {
                s.all_report = serde_json::from_str(include_str!("../fixtures/usage.json"))
                    .expect("demo fixture");
                s.report = s.all_report.clone();
                s.appearance = 1;
                s.error = None;
            }),
            ("empty", |s: &mut State| {
                s.report = Report::default();
                s.all_report = Report::default();
                s.error = None;
            }),
        ],
        setup: Some(|s, wake| {
            s.waker = Some(wake);
            if !rune::is_headless() {
                s.reload();
            }
        }),
        poll: Some(poll),
        fonts: rune_theme_opencode::FONTS,
        families: rune_theme_opencode::FAMILIES,
        on_key: Some(on_key),
        min_size: Some((900.0, 620.0)),
        ..AppSpec::new("Agent Usage", (1120.0, 780.0), State::init, view)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> State {
        let report: Report = serde_json::from_str(include_str!("../fixtures/usage.json")).unwrap();
        State {
            appearance: 2,
            global_scope: true,
            selected: "*".into(),
            report: report.clone(),
            all_report: report,
            error: None,
            range_days: 0,
            pending: None,
            waker: None,
        }
    }
    #[test]
    fn period_reconciles_models_sources_and_totals() {
        let mut s = fixture();
        s.range_days = 7;
        s.apply_period();
        let sources: f64 = s.report.harnesses.values().map(|h| h.cost_usd).sum();
        let models: f64 = s.report.models.values().map(|m| m.cost_usd).sum();
        assert!((sources - s.report.total.cost_usd).abs() < 1e-8);
        assert!((models - sources).abs() < 1e-8);
        assert!(sources > 0.0 && sources < s.all_report.total.cost_usd);
        s.range_days = 0;
        s.apply_period();
        assert_eq!(s.report.total.sessions, 56);
    }
    #[test]
    fn source_selection_filters_summary_and_model_table() {
        let mut s = fixture();
        s.selected = "claude".into();
        assert_eq!(selected_stats(&s).sessions, 9);
        assert_eq!(models(&s).len(), 1);
        assert!(models(&s)[0].0.contains("claude"));
        s.selected = "missing".into();
        assert_eq!(selected_stats(&s).sessions, 0);
        assert!(models(&s).is_empty());
    }
}
