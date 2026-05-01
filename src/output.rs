//! Human vs JSON output helpers. Default is human-readable; `--json`
//! switches every command to compact JSON for piping.

use serde_json::Value;

#[derive(Clone, Copy)]
pub enum Format {
    Human,
    Json,
}

impl Format {
    pub fn from_flag(json: bool) -> Self {
        if json {
            Self::Json
        } else {
            Self::Human
        }
    }
}

pub fn print_state(value: &Value, fmt: Format) {
    match fmt {
        Format::Json => println!("{}", value),
        Format::Human => {
            let app = value.get("app");
            let shell = value.get("shell");
            let pid = app.and_then(|v| v.get("pid")).and_then(|v| v.as_u64());
            let started = app
                .and_then(|v| v.get("started_at_unix_ms"))
                .and_then(|v| v.as_u64());
            let mode = shell
                .and_then(|v| v.get("mode"))
                .and_then(|v| v.as_str())
                .unwrap_or("(none)");
            let route = shell
                .and_then(|v| v.get("route"))
                .and_then(|v| v.as_str())
                .unwrap_or("(none)");

            println!("PA desktop: running");
            if let Some(pid) = pid {
                println!("  pid:     {pid}");
            }
            if let Some(started) = started {
                println!("  started: {started} (unix ms)");
            }
            println!("  mode:    {mode}");
            println!("  route:   {route}");

            print_panes(shell.and_then(|v| v.get("panes")));
        }
    }
}

/// Phase 12 PR-E. The pane snapshot has shape:
///   { leaves: [{ id, focused, activeTabIdx, tabs:[{kind,title}] }],
///     tree:   <opaque PaneNode> }
/// We render the flat leaf list — humans care about "what's open where",
/// not the recursive split structure. The split structure stays in the
/// JSON output for tools that want it.
fn print_panes(panes: Option<&Value>) {
    let Some(panes) = panes else {
        return;
    };
    let Some(leaves) = panes.get("leaves").and_then(|v| v.as_array()) else {
        return;
    };
    if leaves.is_empty() {
        return;
    }
    println!("  panes:   {} open", leaves.len());
    for leaf in leaves {
        let id = leaf.get("id").and_then(|v| v.as_str()).unwrap_or("?");
        let focused = leaf
            .get("focused")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let active = leaf
            .get("activeTabIdx")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let tabs = leaf.get("tabs").and_then(|v| v.as_array());
        let tab_count = tabs.map(|t| t.len()).unwrap_or(0);
        let active_label = tabs
            .and_then(|t| t.get(active))
            .map(|t| {
                let kind = t.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
                let title = t.get("title").and_then(|v| v.as_str()).unwrap_or("");
                if title.is_empty() {
                    kind.to_string()
                } else {
                    format!("{kind} {title}")
                }
            })
            .unwrap_or_else(|| "(empty)".into());
        let marker = if focused { "*" } else { " " };
        let short_id: String = id.chars().take(8).collect();
        println!(
            "    {marker} {short_id}  [{active}/{tab_count}] {active_label}"
        );
    }
}

/// For all the write commands. JSON mode prints the raw response;
/// human mode prints a one-liner confirming what happened.
pub fn print_write_result(label: &str, value: &Value, fmt: Format) {
    match fmt {
        Format::Json => println!("{}", value),
        Format::Human => println!("ok: {label}"),
    }
}
