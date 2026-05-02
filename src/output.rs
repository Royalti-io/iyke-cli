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

pub fn print_dom(value: &Value, fmt: Format) {
    match fmt {
        Format::Json => println!("{}", value),
        Format::Human => {
            if let Some(text) = value.get("text").and_then(|v| v.as_str()) {
                if text.is_empty() {
                    println!("(empty snapshot)");
                } else {
                    println!("{text}");
                }
            } else {
                println!("{}", value);
            }
            if let Some(g) = value.get("generation").and_then(|v| v.as_u64()) {
                eprintln!("# generation {g}");
            }
        }
    }
}

pub fn print_logs(value: &Value, fmt: Format) {
    match fmt {
        Format::Json => println!("{}", value),
        Format::Human => {
            let entries = value
                .get("entries")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if entries.is_empty() {
                println!("(no logs)");
                return;
            }
            for e in entries {
                let ts = e.get("ts").and_then(|v| v.as_u64()).unwrap_or(0);
                let level = e.get("level").and_then(|v| v.as_str()).unwrap_or("?");
                let msg = e.get("message").and_then(|v| v.as_str()).unwrap_or("");
                let src = e.get("source").and_then(|v| v.as_str());
                let prefix = match src {
                    Some(s) => format!("[{ts}] {level:>5} {s}"),
                    None => format!("[{ts}] {level:>5}"),
                };
                println!("{prefix}: {msg}");
            }
        }
    }
}

pub fn print_network(value: &Value, fmt: Format) {
    match fmt {
        Format::Json => println!("{}", value),
        Format::Human => {
            let entries = value
                .get("entries")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if entries.is_empty() {
                println!("(no network activity)");
                return;
            }
            println!("{:>13}  {:<6} {:>4} {:>6}  {}", "ts", "method", "stat", "ms", "url");
            for e in entries {
                let ts = e.get("ts").and_then(|v| v.as_u64()).unwrap_or(0);
                let method = e.get("method").and_then(|v| v.as_str()).unwrap_or("?");
                let url = e.get("url").and_then(|v| v.as_str()).unwrap_or("");
                let status = e
                    .get("status")
                    .and_then(|v| v.as_u64())
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "-".into());
                let dur = e.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0);
                let err = e.get("error").and_then(|v| v.as_str());
                let suffix = err
                    .map(|s| format!("  [error: {s}]"))
                    .unwrap_or_default();
                println!("{ts:>13}  {method:<6} {status:>4} {dur:>6}  {url}{suffix}");
            }
        }
    }
}

pub fn print_screenshot(value: &Value, fmt: Format) {
    match fmt {
        Format::Json => println!("{}", value),
        Format::Human => {
            let path = value.get("path").and_then(|v| v.as_str()).unwrap_or("?");
            let w = value.get("width").and_then(|v| v.as_u64()).unwrap_or(0);
            let h = value.get("height").and_then(|v| v.as_u64()).unwrap_or(0);
            let bytes = value.get("bytesLen").and_then(|v| v.as_u64()).unwrap_or(0);
            println!("ok: {path} ({w}x{h}, {bytes} bytes)");
        }
    }
}

/// Returns true if the wait was satisfied. Caller decides exit code.
pub fn print_wait(value: &Value, fmt: Format) -> bool {
    let satisfied = value
        .get("satisfied")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let elapsed = value.get("elapsed_ms").and_then(|v| v.as_u64()).unwrap_or(0);
    let msg = value.get("message").and_then(|v| v.as_str());
    match fmt {
        Format::Json => println!("{}", value),
        Format::Human => {
            if satisfied {
                println!("ok: satisfied in {elapsed}ms");
            } else {
                let m = msg.unwrap_or("not satisfied");
                println!("timeout after {elapsed}ms: {m}");
            }
        }
    }
    satisfied
}

pub fn print_iframe_state(value: &Value, fmt: Format) {
    match fmt {
        Format::Json => println!("{}", value),
        Format::Human => {
            let pane = value.get("pane").and_then(|v| v.as_str()).unwrap_or("?");
            let gen = value
                .get("generation")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            println!("# pane {pane} (gen {gen})");
            let state = value.get("state");
            match state {
                Some(s) if !s.is_null() => {
                    println!("{}", serde_json::to_string_pretty(s).unwrap_or_default());
                }
                _ => println!(
                    "(no state published — iframe not bridged or no calls to publishState)"
                ),
            }
        }
    }
}

pub fn print_query_cache(value: &Value, fmt: Format) {
    match fmt {
        Format::Json => println!("{}", value),
        Format::Human => {
            let entries = value
                .get("entries")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if entries.is_empty() {
                println!("(empty)");
                return;
            }
            for e in entries {
                let key = e.get("queryKey").map(|v| v.to_string()).unwrap_or_default();
                let status = e.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                let fetch = e.get("fetchStatus").and_then(|v| v.as_str()).unwrap_or("?");
                let stale = e.get("isStale").and_then(|v| v.as_bool()).unwrap_or(false);
                let upd = e
                    .get("dataUpdatedAt")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let preview = e
                    .get("dataPreview")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let err = e.get("error").and_then(|v| v.as_str());
                let stale_marker = if stale { "*stale*" } else { "fresh" };
                let line = format!(
                    "{status:>8} {fetch:>8} {stale_marker:>8}  upd={upd}  {key}"
                );
                println!("{line}");
                if !preview.is_empty() {
                    println!("    data: {preview}");
                }
                if let Some(e) = err {
                    println!("    error: {e}");
                }
            }
        }
    }
}
