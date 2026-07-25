# iyke

[![Version](https://img.shields.io/badge/version-v0.0.0-blue.svg)](https://github.com/Royalti-io/iyke-cli/releases)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

> `iyke` — the runtime controller for a running Ikenga shell. Drive panes, modes, tabs, and
> read live state from your terminal.

## What it is

`iyke` talks to a *running* Ikenga desktop app over its localhost control bridge. The app
binds an HTTP server to `127.0.0.1:<random-port>` with a per-launch bearer token; `iyke`
reads the control file the app writes and forwards subcommands as authenticated calls. Use
it to navigate, switch modes, open tabs, and inspect UI state from a script.

It's one of two Ikenga CLIs — the **runtime** one. (The other,
[`ikenga`](https://github.com/Royalti-io/ikenga-cli), manages packages on disk. They share
no code.)

## Install

From the monorepo root:

```bash
cargo install --path iyke-cli
```

This puts an `iyke` binary in `~/.cargo/bin`. Make sure that directory is on your `PATH`.

## Examples

```bash
iyke state                           # show what the app is currently displaying
iyke --json state | jq .shell.route  # programmatic access

iyke go /finance/receivables         # navigate the focused pane
iyke mode files                      # switch the sidebar to the files mode

iyke open route /agents              # open a new tab in the focused pane
iyke open terminal --cmd "bun run dev"
iyke open mini-app video-engine

iyke split horizontal                # split the focused pane side-by-side
iyke focus index 2                   # focus the 2nd leaf pane (matches ⌃2)
iyke close                           # close the focused pane

# Kernel-authoritative terminal orchestration
TERMINAL=$(iyke --json terminals | jq -r '.[0].terminal_id')
iyke terminal-label "$TERMINAL" reviewer
iyke terminal-read --label reviewer --after 0
CURSOR=$(iyke --json terminal-read --label reviewer | jq -r .end_offset)
iyke terminal-send "run checks" --label reviewer --key Enter --no-focus --actor orchestrator
iyke terminal-wait --label reviewer --after "$CURSOR" --match 'READY|FAILED'
iyke terminal-read --label reviewer --mode screen

# Optional writer exclusion and restart guards
LEASE=$(iyke --json terminal-lease acquire "$TERMINAL" --agent-id orchestrator | jq -r .token)
PTY=$(iyke --json terminals | jq -r --arg id "$TERMINAL" '.[] | select(.terminal_id == $id) | .pty_id')
iyke terminal-send "continue" --terminal "$TERMINAL" --key Enter \
  --expected-pty-id "$PTY" --lease-token "$LEASE" --actor orchestrator

iyke terminal-lease release "$TERMINAL" --token "$LEASE"

# Shared coordination store with event-driven long polling

iyke scratchpad set --scope project:royalti-co --name handoff --body ready
iyke scratchpad watch --scope project:royalti-co --name handoff
```

Add `--json` to any command for machine-readable output. If the desktop app is not running,
every command exits non-zero with a clear message instead of hanging.

## Links

- [`ikenga-cli`](https://github.com/Royalti-io/ikenga-cli) — the package manager (the *other* CLI)
- [`ikenga`](https://github.com/Royalti-io/ikenga) — the desktop shell it controls

## License

Apache-2.0 — see [`LICENSE`](LICENSE).
