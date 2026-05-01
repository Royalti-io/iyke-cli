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
    }

    Ok(())
}
