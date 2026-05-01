# iyke

CLI for the Royalti PA desktop app's localhost control bridge. The desktop app exposes an HTTP server bound to `127.0.0.1:<random-port>` with a per-launch bearer token; this CLI reads the control file the app writes and forwards subcommands as authenticated HTTP calls. Use it to navigate panes, switch sidebar modes, open tabs, and inspect state from a terminal or script.

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
iyke open mini-app storyboard

iyke split horizontal                # split the focused pane side-by-side
iyke focus index 2                   # focus the 2nd leaf pane (matches ⌃2)
iyke close                           # close the focused pane
```

Add `--json` to any command for machine-readable output.

If the desktop app is not running, every command exits non-zero with a clear message instead of hanging.
