//! `iyke` — CLI for the Royalti PA desktop app's localhost control bridge.
//!
//! The desktop app exposes an HTTP server bound to `127.0.0.1:<random>`
//! with a per-launch bearer token. The token + port are written to a
//! control file under the user's local data dir; this CLI reads that
//! file and proxies subcommands as authenticated HTTP calls.
//!
//! Subcommands roughly mirror what the in-app FE can do via keyboard
//! shortcuts: navigate the focused pane, switch sidebar mode, open new
//! tabs, split/focus/close panes.

mod api;
mod control;
mod output;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand, ValueEnum};
use serde_json::json;

use crate::api::Client;
use crate::control::{LoadOutcome, STALE_THRESHOLD_SECS};
use crate::output::{print_state, print_write_result, Format};

#[derive(Parser)]
#[command(
    name = "iyke",
    version,
    about = "Control the Royalti PA desktop app from outside the webview.",
    long_about = "iyke talks to the localhost control bridge that the PA desktop app exposes. \
                  Use it to navigate panes, switch sidebar modes, open tabs, and inspect state \
                  from a terminal or script."
)]
struct Cli {
    /// Emit JSON instead of human-readable output.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the current shell state (mode, focused route, app info).
    State,

    /// Navigate the focused pane to a route path (e.g. `/finance/receivables`).
    Go {
        /// Route path. Must start with `/`.
        path: String,
    },

    /// Switch the sidebar activity mode.
    Mode {
        /// One of: app, files, agents, sessions, settings, storyboard,
        /// video-engine, canvas-design, image-generator.
        mode: String,
    },

    /// Open a new tab in the focused pane.
    Open {
        #[command(subcommand)]
        kind: OpenKind,
    },

    /// Split the focused (or specified) pane.
    Split {
        /// Direction.
        direction: SplitDirection,
        /// Optional pane id to split. Defaults to focused.
        #[arg(long)]
        pane_id: Option<String>,
    },

    /// Focus a pane by id or DFS leaf index (1-based, matching ⌃1..⌃6).
    Focus {
        #[command(subcommand)]
        target: FocusTarget,
    },

    /// Close a pane (or the focused pane if id omitted).
    Close {
        /// Optional pane id. Defaults to focused.
        #[arg(long)]
        pane_id: Option<String>,
    },

    /// Resize the main window. Pass either `<W>x<H>` (e.g. `1600x1000`) or
    /// a preset: `maximize`, `unmaximize`, `fullscreen`, `unfullscreen`,
    /// `minimize`.
    Resize {
        /// `<W>x<H>` or a preset name.
        target: String,
    },

    /// Refresh a pane's content (re-mount via React key bump). Defaults to
    /// the focused pane.
    Refresh {
        /// Optional pane id. Defaults to focused.
        #[arg(long)]
        pane_id: Option<String>,
    },

    /// Print an accessibility-tree snapshot of the focused pane (or `--pane`).
    /// Refs (e.g. `e3`) are stable until the next snapshot or navigation.
    Dom {
        /// Substring filter; only entries whose role/name/value match are kept.
        #[arg(long)]
        query: Option<String>,
        /// Include hidden + aria-hidden + zero-size elements.
        #[arg(long)]
        all: bool,
        /// Pane id. Default = focused.
        #[arg(long)]
        pane: Option<String>,
    },

    /// Print recent console + error logs from the running webview.
    Logs {
        /// Filter by level: log | info | warn | error | debug.
        #[arg(long)]
        level: Option<String>,
        /// Only entries with `ts >= since` (epoch ms).
        #[arg(long)]
        since: Option<u128>,
        /// Filter by source pane id (e.g. `shell` or a leaf id).
        #[arg(long)]
        source: Option<String>,
    },

    /// Print recent fetch + XHR network activity (last 100).
    Network {
        #[arg(long)]
        since: Option<u128>,
        #[arg(long)]
        source: Option<String>,
    },

    /// Capture a screenshot of the focused pane or the full window.
    Screenshot {
        /// Capture target.
        #[arg(value_enum, default_value = "window")]
        target: ScreenshotTarget,
        /// Output path. Default: ~/.local/share/royalti-pa/screenshots/<auto>.png.
        #[arg(long)]
        out: Option<String>,
        /// Pane id when target=pane.
        #[arg(long)]
        pane_id: Option<String>,
    },

    /// Wait until a predicate is satisfied or timeout. Exit non-zero on timeout.
    Wait {
        /// Predicate kind: text | selector | ref | gone-text | gone-selector.
        kind: String,
        /// Predicate value.
        value: String,
        /// Timeout in milliseconds (default 10000, max 60000).
        #[arg(long)]
        timeout_ms: Option<u64>,
        #[arg(long)]
        pane: Option<String>,
    },

    /// Click an element. Specify exactly one of `--ref`, `--selector`, `--text`.
    Click {
        #[arg(long)]
        r#ref: Option<String>,
        #[arg(long)]
        selector: Option<String>,
        #[arg(long)]
        text: Option<String>,
        #[arg(long)]
        pane: Option<String>,
    },

    /// Type text into an input/textarea/contenteditable.
    Type {
        /// Text to type.
        text: String,
        #[arg(long)]
        r#ref: Option<String>,
        #[arg(long)]
        selector: Option<String>,
        /// Replace the existing value instead of appending.
        #[arg(long)]
        replace: bool,
        #[arg(long)]
        pane: Option<String>,
    },

    /// Dispatch a key combo (e.g. `Enter`, `Ctrl+S`, `Meta+K`).
    Key {
        /// Combo string. `+` or `,` separated.
        combo: String,
        #[arg(long)]
        r#ref: Option<String>,
        #[arg(long)]
        selector: Option<String>,
        #[arg(long)]
        pane: Option<String>,
    },

    /// Dump the TanStack Query cache: keys, statuses, last update times.
    QueryCache {
        #[arg(long)]
        pane: Option<String>,
    },

    /// Open Chrome DevTools (debug builds only).
    Devtools,

    /// Read the latest published state object for an iframe pane (storyboard
    /// cursor, comp current frame, etc.). Iframes publish via the bridge's
    /// `publishState(key, value)` API.
    IframeState {
        /// Pane id (from `iyke state` shell.panes.leaves[].id).
        pane: String,
    },

    /// Send a fire-and-forget postMessage to an iframe pane. Used to drive
    /// mini-app actions from the terminal — e.g.
    /// `iyke iframe-send <pane> story-select '{"beatId":"hook"}'`.
    IframeSend {
        /// Pane id.
        pane: String,
        /// Message kind. Up to the iframe to interpret.
        kind: String,
        /// JSON payload. Defaults to null.
        #[arg(default_value = "null")]
        payload: String,
    },
}

#[derive(Copy, Clone, ValueEnum)]
enum ScreenshotTarget {
    Window,
    Pane,
}

#[derive(Subcommand)]
enum OpenKind {
    /// Open a route view at `path`.
    Route { path: String },
    /// Open a fresh terminal session.
    Terminal {
        /// Optional command (joined with spaces for the shell). Defaults to login shell.
        #[arg(long)]
        cmd: Option<String>,
    },
    /// Open a chat session by id (or "new" to start one — server-side decides).
    Chat { session_id: String },
    /// Open a file artifact viewer.
    Artifact { path: String },
    /// Open a mini-app by name (storyboard, video-engine, canvas-design, image-generator).
    MiniApp { name: String },
}

#[derive(Subcommand)]
enum FocusTarget {
    /// Focus by leaf id.
    Id { pane_id: String },
    /// Focus by 1-based DFS index, like ⌃1..⌃6 in-app.
    Index { index: u8 },
}

#[derive(Copy, Clone, ValueEnum)]
enum SplitDirection {
    Horizontal,
    Vertical,
}

impl SplitDirection {
    fn as_str(self) -> &'static str {
        match self {
            Self::Horizontal => "horizontal",
            Self::Vertical => "vertical",
        }
    }
}

/// `1600x1000` → `(label, json)` for explicit size; preset keyword →
/// `(label, { "preset": <kw> })`. Anything else is a hard error so users
/// see a typo immediately rather than getting a server-side 400.
fn parse_resize_target(target: &str) -> Result<(String, serde_json::Value)> {
    const PRESETS: &[&str] = &[
        "maximize",
        "unmaximize",
        "fullscreen",
        "unfullscreen",
        "minimize",
    ];
    if PRESETS.contains(&target) {
        return Ok((
            format!("resize {target}"),
            json!({ "preset": target }),
        ));
    }
    if let Some((w, h)) = target.split_once('x') {
        let w: u32 = w
            .parse()
            .map_err(|_| anyhow!("invalid width in {target:?}: expected integer"))?;
        let h: u32 = h
            .parse()
            .map_err(|_| anyhow!("invalid height in {target:?}: expected integer"))?;
        return Ok((
            format!("resize {w}x{h}"),
            json!({ "width": w, "height": h }),
        ));
    }
    Err(anyhow!(
        "could not parse resize target {target:?}: expected `<W>x<H>` or one of {PRESETS:?}"
    ))
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let fmt = Format::from_flag(cli.json);

    let cf = match control::load()? {
        LoadOutcome::Ok(cf) => cf,
        LoadOutcome::Missing => {
            return Err(anyhow!(
                "PA desktop app does not appear to be running (no control.json found)."
            ));
        }
        LoadOutcome::StaleRemoved => {
            return Err(anyhow!(
                "PA desktop app does not appear to be running (cleared a stale control.json from a previous launch)."
            ));
        }
        LoadOutcome::StaleYoung { age_secs } => {
            return Err(anyhow!(
                "control.json exists but the recorded PID is dead and the file is only {age_secs}s old \
                 (threshold {STALE_THRESHOLD_SECS}s). The app may be launching or in a startup race; \
                 retry shortly, or delete the file by hand if you're sure it's stale."
            ));
        }
    };

    let client = Client::new(&cf);

    match cli.command {
        Command::State => {
            let v = client.get_state()?;
            print_state(&v, fmt);
        }
        Command::Go { path } => {
            if !path.starts_with('/') {
                return Err(anyhow!("path must start with '/' (got {path:?})"));
            }
            let v = client.post("/iyke/go", json!({ "path": path }))?;
            print_write_result(&format!("go {path}"), &v, fmt);
        }
        Command::Mode { mode } => {
            let v = client.post("/iyke/mode", json!({ "mode": mode }))?;
            print_write_result(&format!("mode {mode}"), &v, fmt);
        }
        Command::Open { kind } => {
            let (label, body) = match kind {
                OpenKind::Route { path } => (
                    format!("open route {path}"),
                    json!({ "kind": "route", "path": path }),
                ),
                OpenKind::Terminal { cmd } => (
                    format!("open terminal{}", cmd.as_deref().map(|c| format!(" ({c})")).unwrap_or_default()),
                    json!({ "kind": "terminal", "cmd": cmd }),
                ),
                OpenKind::Chat { session_id } => (
                    format!("open chat {session_id}"),
                    json!({ "kind": "chat", "session_id": session_id }),
                ),
                OpenKind::Artifact { path } => (
                    format!("open artifact {path}"),
                    json!({ "kind": "artifact", "path": path }),
                ),
                OpenKind::MiniApp { name } => (
                    format!("open mini-app {name}"),
                    json!({ "kind": "mini-app", "name": name }),
                ),
            };
            let v = client.post("/iyke/open", body)?;
            print_write_result(&label, &v, fmt);
        }
        Command::Split { direction, pane_id } => {
            let body = match pane_id {
                Some(id) => json!({ "direction": direction.as_str(), "pane_id": id }),
                None => json!({ "direction": direction.as_str() }),
            };
            let v = client.post("/iyke/split", body)?;
            print_write_result(&format!("split {}", direction.as_str()), &v, fmt);
        }
        Command::Focus { target } => {
            let (label, body) = match target {
                FocusTarget::Id { pane_id } => (
                    format!("focus {pane_id}"),
                    json!({ "pane_id": pane_id }),
                ),
                FocusTarget::Index { index } => (
                    format!("focus index {index}"),
                    json!({ "index": index }),
                ),
            };
            let v = client.post("/iyke/focus", body)?;
            print_write_result(&label, &v, fmt);
        }
        Command::Resize { target } => {
            let (label, body) = parse_resize_target(&target)?;
            let v = client.post("/iyke/resize", body)?;
            print_write_result(&label, &v, fmt);
        }
        Command::Refresh { pane_id } => {
            let body = match pane_id.as_ref() {
                Some(id) => json!({ "pane_id": id }),
                None => json!({}),
            };
            let v = client.post("/iyke/refresh", body)?;
            print_write_result(
                &format!("refresh{}", pane_id.map(|id| format!(" {id}")).unwrap_or_default()),
                &v,
                fmt,
            );
        }
        Command::Close { pane_id } => {
            let body = match pane_id {
                Some(ref id) => json!({ "pane_id": id }),
                None => json!({}),
            };
            let v = client.post("/iyke/close", body)?;
            print_write_result(
                &format!("close{}", pane_id.map(|id| format!(" {id}")).unwrap_or_default()),
                &v,
                fmt,
            );
        }
        Command::Dom { query, all, pane } => {
            let mut q = Vec::new();
            if let Some(s) = &query {
                q.push(("query", s.clone()));
            }
            if all {
                q.push(("all", "true".into()));
            }
            if let Some(p) = &pane {
                q.push(("pane", p.clone()));
            }
            let v = client.get_with_query("/iyke/dom", &q)?;
            output::print_dom(&v, fmt);
        }
        Command::Logs { level, since, source } => {
            let mut q = Vec::new();
            if let Some(s) = &level {
                q.push(("level", s.clone()));
            }
            if let Some(s) = since {
                q.push(("since", s.to_string()));
            }
            if let Some(s) = &source {
                q.push(("source", s.clone()));
            }
            let v = client.get_with_query("/iyke/logs", &q)?;
            output::print_logs(&v, fmt);
        }
        Command::Network { since, source } => {
            let mut q = Vec::new();
            if let Some(s) = since {
                q.push(("since", s.to_string()));
            }
            if let Some(s) = &source {
                q.push(("source", s.clone()));
            }
            let v = client.get_with_query("/iyke/network", &q)?;
            output::print_network(&v, fmt);
        }
        Command::Screenshot { target, out, pane_id } => {
            let path = match target {
                ScreenshotTarget::Window => "/iyke/screenshot/window",
                ScreenshotTarget::Pane => "/iyke/screenshot/pane",
            };
            let mut body = serde_json::Map::new();
            if let Some(p) = out {
                body.insert("out_path".into(), json!(p));
            }
            if matches!(target, ScreenshotTarget::Pane) {
                let id = pane_id
                    .ok_or_else(|| anyhow!("--pane-id required when target=pane"))?;
                body.insert("pane_id".into(), json!(id));
            }
            let v = client.post(path, serde_json::Value::Object(body))?;
            output::print_screenshot(&v, fmt);
        }
        Command::Wait { kind, value, timeout_ms, pane } => {
            let body = json!({
                "kind": kind,
                "value": value,
                "timeout_ms": timeout_ms,
                "pane": pane,
            });
            let v = client.post("/iyke/wait", body)?;
            let satisfied = output::print_wait(&v, fmt);
            if !satisfied {
                std::process::exit(2);
            }
        }
        Command::Click { r#ref, selector, text, pane } => {
            require_one(&r#ref, &selector, &text)?;
            let body = json!({
                "ref": r#ref,
                "selector": selector,
                "text": text,
                "pane": pane,
            });
            let v = client.post("/iyke/click", body)?;
            print_write_result("click", &v, fmt);
        }
        Command::Type { text, r#ref, selector, replace, pane } => {
            require_one(&r#ref, &selector, &None)?;
            let body = json!({
                "ref": r#ref,
                "selector": selector,
                "text": text,
                "replace": replace,
                "pane": pane,
            });
            let v = client.post("/iyke/type", body)?;
            print_write_result("type", &v, fmt);
        }
        Command::Key { combo, r#ref, selector, pane } => {
            let body = json!({
                "combo": combo,
                "ref": r#ref,
                "selector": selector,
                "pane": pane,
            });
            let v = client.post("/iyke/key", body)?;
            print_write_result(&format!("key {combo}"), &v, fmt);
        }
        Command::QueryCache { pane } => {
            let mut q = Vec::new();
            if let Some(p) = &pane {
                q.push(("pane", p.clone()));
            }
            let v = client.get_with_query("/iyke/query-cache", &q)?;
            output::print_query_cache(&v, fmt);
        }
        Command::Devtools => {
            let v = client.post("/iyke/devtools", json!({}))?;
            print_write_result("devtools", &v, fmt);
        }
        Command::IframeState { pane } => {
            let v = client.get_with_query("/iyke/iframe-state", &[("pane", pane.clone())])?;
            output::print_iframe_state(&v, fmt);
        }
        Command::IframeSend { pane, kind, payload } => {
            let parsed: serde_json::Value = serde_json::from_str(&payload)
                .map_err(|e| anyhow!("invalid payload JSON: {e}"))?;
            let v = client.post(
                "/iyke/iframe-message",
                json!({ "pane": pane, "kind": kind, "payload": parsed }),
            )?;
            print_write_result(&format!("iframe-send {pane} {kind}"), &v, fmt);
        }
    }

    Ok(())
}

fn require_one(
    r#ref: &Option<String>,
    selector: &Option<String>,
    text: &Option<String>,
) -> Result<()> {
    let count = r#ref.is_some() as u8 + selector.is_some() as u8 + text.is_some() as u8;
    if count != 1 {
        return Err(anyhow!(
            "must supply exactly one of: --ref, --selector, --text"
        ));
    }
    Ok(())
}
