//! agent-usage native UI built with Rune + OpenCode theme.
//!
//! Design: Rune ships no brand — core is unbranded tokens, and the OpenCode
//! theme (`rune-theme-opencode`, generated from `@opencode/ui`) is the first
//! theme and reference example. This app uses exactly that: `kit(dark)` for
//! colours/type/radius/motion, Inter + IBM Plex Mono fonts, same look on
//! every platform (PLAN.md §7).

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

use rune::{AppSpec, Cx, El, col, el, row, text};
use rune_kit::{Badge, Button, Kit};

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
    reasoning_tokens: u64,
    #[serde(default)]
    cost_usd: f64,
    #[serde(default)]
    unpriced: bool,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct Harness {
    #[serde(default)]
    store: String,
    #[serde(default)]
    found: bool,
    #[serde(default)]
    note: String,
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
    scope: Option<String>,
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
    use std::hash::{Hash, Hasher as _};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    SCRIPT.hash(&mut h);
    PRICING.hash(&mut h);
    std::hash::Hasher::finish(&h)
}

fn load_report(global: bool) -> Result<Report, String> {
    let script = cache_script();
    let mut args = vec![
        script.to_string_lossy().to_string(),
        "--json".to_string(),
    ];
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

// ---------------------------------------------------------------- state

struct State {
    appearance: u8, // 0 auto, 1 light, 2 dark
    global_scope: bool,
    selected: String,
    report: Report,
    error: Option<String>,
}

impl State {
    fn init() -> Self {
        let global_scope = true;
        let (report, error) = match load_report(global_scope) {
            Ok(r) => (r, None),
            Err(e) => (Report::default(), Some(e)),
        };
        State {
            appearance: 0,
            global_scope,
            selected: "*".to_string(),
            report,
            error,
        }
    }

    fn reload(&mut self) {
        match load_report(self.global_scope) {
            Ok(r) => {
                self.report = r;
                self.error = None;
            }
            Err(e) => self.error = Some(e),
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

// ---------------------------------------------------------------- view

fn view(s: &State, cx: &mut Cx<State>) -> El {
    let dark = s.dark(cx.system_dark());
    let k = rune_theme_opencode::kit(dark);
    let mut overlays = Vec::new();
    let body = col()
        .key("window")
        .w_full()
        .h_full()
        .fill(k.c.page)
        .child(header(s, k))
        .child(tiles(s, k))
        .child(main_body(s, k))
        .child(footer(s, k));
    rune_kit::root(body, overlays.drain(..))
}

fn header(s: &State, k: &'static Kit) -> El {
    let scope_label = if s.global_scope {
        "all directories".to_string()
    } else {
        "this directory + subdirs".to_string()
    };
    let appearance_label = match s.appearance {
        1 => "Light",
        2 => "Dark",
        _ => "Auto",
    };
    row()
        .key("header")
        .w_full()
        .h(52.0)
        .px(16.0)
        .center_y()
        .justify_between()
        .fill(k.c.bg_base)
        .child(
            col().key("title-col").gap(2.0).child(
                text("agent-usage")
                    .key("title")
                    .font(k.ty.ui_medium)
                    .color(k.c.text_base),
            ).child(
                text(scope_label)
                    .key("scope")
                    .font(k.ty.small)
                    .color(k.c.text_muted),
            ),
        )
        .child(
            row().key("actions").gap(8.0).center_y()
                .child(
                    Button::new(if s.global_scope { "Global" } else { "Dir" })
                        .on_press(|s: &mut State| {
                            s.global_scope = !s.global_scope;
                            s.reload();
                        })
                        .build(k)
                        .key("scope-btn"),
                )
                .child(
                    Button::new(appearance_label)
                        .on_press(|s: &mut State| {
                            s.appearance = (s.appearance + 1) % 3;
                        })
                        .build(k)
                        .key("theme-btn"),
                )
                .child(
                    Button::new("Refresh")
                        .on_press(|s: &mut State| s.reload())
                        .build(k)
                        .key("refresh-btn"),
                ),
        )
}

fn tile(k: &'static Kit, key: &str, title: &str, value: String, sub: String) -> El {
    col()
        .key(key)
        .grow(1.0)
        .gap(2.0)
        .padding(12.0)
        .radius(k.radius.lg)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
        .child(text(title).key(format!("{key}-t")).font(k.ty.small).color(k.c.text_muted))
        .child(text(value).key(format!("{key}-v")).font(k.ty.body_medium).color(k.c.text_base))
        .child(text(sub).key(format!("{key}-s")).font(k.ty.small).color(k.c.text_faint))
}

fn tiles(s: &State, k: &'static Kit) -> El {
    let t = &s.report.total;
    let tokens = t.input_tokens + t.output_tokens + t.cache_read_tokens + t.cache_write_tokens;
    row()
        .key("tiles")
        .w_full()
        .px(16.0)
        .py(12.0)
        .gap(12.0)
        .child(tile(k, "cost", "API cost", money(t.cost_usd), format!("{} models", s.report.models.len())))
        .child(tile(k, "sess", "Sessions", format!("{}", t.sessions), format!("{} msgs", t.messages)))
        .child(tile(k, "msgs", "Messages", format!("{}", t.messages), format!("{} sess", t.sessions)))
        .child(tile(
            k,
            "toks",
            "Tokens",
            compact(tokens),
            format!("in {} · out {}", compact(t.input_tokens), compact(t.output_tokens)),
        ))
}

fn main_body(s: &State, k: &'static Kit) -> El {
    row()
        .key("body")
        .w_full()
        .grow(1.0)
        .px(16.0)
        .pb(12.0)
        .gap(12.0)
        .child(harness_list(s, k))
        .child(model_table(s, k))
}

fn harness_list(s: &State, k: &'static Kit) -> El {
    let mut names: Vec<(&String, &Harness)> = s.report.harnesses.iter().collect();
    names.sort_by(|a, b| b.1.cost_usd.partial_cmp(&a.1.cost_usd).unwrap());
    let mut list = col().key("harness-items").gap(4.0);
    let all_sel = s.selected == "*";
    list = list.child(harness_row(
        k,
        "all",
        "All harnesses",
        &format!("{} · {} sess", money(s.report.total.cost_usd), s.report.total.sessions),
        all_sel,
        "*".to_string(),
    ));
    for (name, h) in names {
        if h.sessions == 0 && !h.found {
            continue;
        }
        let sub = format!("{} · {} sess", money(h.cost_usd), h.sessions);
        list = list.child(harness_row(k, name, name, &sub, s.selected == *name, name.clone()));
    }
    col()
        .key("harnesses")
        .w(248.0)
        .h_full()
        .gap(8.0)
        .padding(12.0)
        .radius(k.radius.lg)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
        .child(text("Harness").key("h-title").font(k.ty.small_medium).color(k.c.text_muted))
        .child(list)
}

fn harness_row(k: &'static Kit, key: &str, name: &str, sub: &str, selected: bool, value: String) -> El {
    el()
        .key(format!("h-{key}"))
        .role("button")
        .w_full()
        .padding(8.0)
        .radius(k.radius.md)
        .fill(if selected { k.c.hover } else { rune::Color::TRANSPARENT })
        .hover(k.c.hover)
        .on_press(move |s: &mut State| s.selected = value.clone())
        .child(
            col().key(format!("h-{key}-c")).gap(2.0)
                .child(text(name.to_string()).key(format!("h-{key}-n")).font(k.ty.ui_medium).color(k.c.text_base))
                .child(text(sub.to_string()).key(format!("h-{key}-s")).font(k.ty.small).color(k.c.text_muted)),
        )
}

fn model_table(s: &State, k: &'static Kit) -> El {
    let models: Vec<(&String, &Stats)> = if s.selected == "*" {
        s.report.models.iter().collect()
    } else {
        s.report
            .harnesses
            .get(&s.selected)
            .map(|h| h.by_model.iter().collect())
            .unwrap_or_default()
    };
    let mut rows: Vec<(&String, &Stats)> = models;
    rows.sort_by(|a, b| b.1.cost_usd.partial_cmp(&a.1.cost_usd).unwrap());
    rows.truncate(14);

    let mut list = col().key("model-rows").gap(2.0);
    if rows.is_empty() {
        let msg = s.error.clone().unwrap_or_else(|| "No sessions in scope.".to_string());
        list = list.child(
            text(msg).key("empty").font(k.ty.ui).color(k.c.text_muted),
        );
    }
    for (name, st) in rows {
        let unpriced = st.unpriced && st.cost_usd == 0.0;
        list = list.child(
            row()
                .key(format!("m-{name}"))
                .w_full()
                .px(8.0)
                .py(6.0)
                .center_y()
                .justify_between()
                .radius(k.radius.sm)
                .hover(k.c.hover)
                .child(
                    col().key(format!("m-{name}-l")).gap(1.0).grow(1.0)
                        .child(text(name.to_string()).key(format!("m-{name}-n")).font(k.ty.ui).color(k.c.text_base))
                        .child(
                            text(format!(
                                "{} sess · {} in · {} out",
                                st.sessions,
                                compact(st.input_tokens),
                                compact(st.output_tokens)
                            ))
                            .key(format!("m-{name}-s"))
                            .font(k.ty.small)
                            .color(k.c.text_faint),
                        ),
                )
                .child(
                    if unpriced {
                        Badge::new("unpriced").build(k).key(format!("m-{name}-b"))
                    } else {
                        text(money(st.cost_usd))
                            .key(format!("m-{name}-c"))
                            .font(k.ty.ui_medium)
                            .color(k.c.text_base)
                    },
                ),
        );
    }
    col()
        .key("models")
        .grow(1.0)
        .h_full()
        .gap(8.0)
        .padding(12.0)
        .radius(k.radius.lg)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
        .child(
            row().key("m-head").w_full().justify_between().center_y()
                .child(text(if s.selected == "*" { "Models".to_string() } else { s.selected.clone() }).key("m-title").font(k.ty.small_medium).color(k.c.text_muted))
                .child(text(format!("{} shown", rows_len(s))).key("m-count").font(k.ty.small).color(k.c.text_faint)),
        )
        .child(list)
}

fn rows_len(s: &State) -> usize {
    if s.selected == "*" {
        s.report.models.len().min(14)
    } else {
        s.report
            .harnesses
            .get(&s.selected)
            .map(|h| h.by_model.len().min(14))
            .unwrap_or(0)
    }
}

fn footer(s: &State, k: &'static Kit) -> El {
    let msg = if let Some(e) = &s.error {
        e.clone()
    } else {
        "g scope · r refresh · t theme — python3 required (brew install python)".to_string()
    };
    row()
        .key("footer")
        .w_full()
        .h(28.0)
        .px(16.0)
        .center_y()
        .fill(k.c.bg_base)
        .child(text(msg).key("hint").font(k.ty.small).color(k.c.text_faint))
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
            ("empty", |s: &mut State| {
                s.report = Report::default();
                s.error = None;
            }),
        ],
        fonts: rune_theme_opencode::FONTS,
        families: rune_theme_opencode::FAMILIES,
        on_key: Some(on_key),
        ..AppSpec::new("Agent Usage", (980.0, 660.0), State::init, view)
    });
}
