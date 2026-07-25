#!/usr/bin/env bash
#
# Live end-to-end test against a RUNNING Ikenga shell.
#
# The unit tests in src/main.rs only prove clap parses the flags. This
# exercises the actual wire: real PTYs, the frontend spawn round-trip, real
# leases, and the timer -> inbox delivery path. Run it after any change to
# the terminal / inbox / task command surface, and after a shell-side change
# to the endpoints they call.
#
#   bun run tauri dev          # in the shell repo, first
#   ./scripts/live-test.sh
#
# Binary resolution: $IYKE, else target/release/iyke, else target/debug/iyke,
# else whatever `iyke` is on PATH.
#
# Exits non-zero if any check fails.
#
# KNOWN: check 4 (duplicate label) fails roughly 1 run in 5 against a dev
# shell. That is a true positive — a real race in the shell's duplicate-label
# guard, documented at the check itself. Don't weaken the assertion to get a
# green run; fix the guard.
#
# Leaves behind (no delete endpoint exists for either, both harmless):
#   - one registered agent, `live-orch`
#   - one completed task per run, titled "[live-test] control-plane check"
# Terminals and their tabs ARE cleaned up.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [ -n "${IYKE:-}" ]; then :
elif [ -x "$REPO_ROOT/target/release/iyke" ]; then IYKE="$REPO_ROOT/target/release/iyke"
elif [ -x "$REPO_ROOT/target/debug/iyke" ]; then IYKE="$REPO_ROOT/target/debug/iyke"
else IYKE="iyke"; fi
command -v "$IYKE" >/dev/null 2>&1 || [ -x "$IYKE" ] || {
  echo "no iyke binary — build one (cargo build --release) or set \$IYKE"; exit 1; }

CTL="${XDG_DATA_HOME:-$HOME/.local/share}/app.ikenga/control.json"
# Labels are run-scoped so a previous aborted run can't collide with this one.
RUN="lt-$$"
AGENT="live-orch"

PASS=0; FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS+1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL+1)); }
hdr() { echo; echo "── $* ─────────────────────────────"; }

# Terminal ids this run created, so cleanup can verify by id rather than by a
# label whose descriptor may already be gone.
SPAWNED=()

# True while a tab with this terminal id is still in the pane tree.
tab_present() {
  "$IYKE" --json state 2>/dev/null \
    | jq -e --arg t "$1" '[.shell.panes.leaves[].tabs[]
        | select(.kind=="terminal" and .terminalId==$t)] | length > 0' >/dev/null 2>&1
}

cleanup() {
  # Exited descriptors linger ~10min in terminal-list by design; the TABS are
  # what actually clutter the UI, so those are what we verify.
  #
  # `--close-tab` emits `iyke://terminal-close-tab` fire-and-forget (the process
  # is already dead, so the shell doesn't wait for the frontend to ack). A tab
  # therefore occasionally survives a single request — observed once across two
  # runs. Re-issue until the tab is actually gone rather than assuming it went.
  for tid in ${SPAWNED+"${SPAWNED[@]}"}; do
    for _ in $(seq 1 5); do
      tab_present "$tid" || break
      "$IYKE" terminal-kill "$tid" --close-tab >/dev/null 2>&1 || true
      sleep 0.4
    done
    if tab_present "$tid"; then
      echo "  WARN: leaked terminal tab $tid — close it by hand" >&2
    fi
  done
}
trap cleanup EXIT

hdr "0. shell reachable + frontend ready"
"$IYKE" state >/dev/null 2>&1 && ok "bridge answers" || { bad "bridge unreachable — is the shell running?"; exit 1; }
# control.json is written during Rust setup, BEFORE React mounts and registers
# the iyke:// listeners. Until it does, every frontend-backed endpoint (spawn,
# dom, click) 503s with "timed out after 10000ms" while the DB-backed ones
# answer fine. Gate on a real frontend round-trip, not on the control file.
READY=no
for _ in $(seq 1 60); do
  if "$IYKE" dom >/dev/null 2>&1; then READY=yes; break; fi
  sleep 1
done
[ "$READY" = yes ] && ok "frontend listeners registered" \
  || { bad "frontend never became ready after 60s"; exit 1; }

hdr "1. agent register"
AGENT_JSON=$("$IYKE" --json agent register --id "$AGENT" --name "Live Orchestrator" 2>&1)
jq -e --arg a "$AGENT" '.id==$a and has("registered_at")' <<<"$AGENT_JSON" >/dev/null \
  && ok "registered" || bad "register: $AGENT_JSON"

hdr "2. terminal-spawn with label + lease (real pty)"
SPAWN=$("$IYKE" --json terminal-spawn --label "$RUN-a" --lease-for "$AGENT" \
        --cwd /tmp -- bash --norc 2>&1)
TID=$(jq -r '.terminal_id // empty' <<<"$SPAWN" 2>/dev/null)
[ -n "$TID" ] && { ok "terminal_id minted"; SPAWNED+=("$TID"); } || bad "no terminal_id: $SPAWN"
PTY=$(jq -r '.pty_id // empty' <<<"$SPAWN" 2>/dev/null)
[ -n "$PTY" ] && ok "pty_id present" || bad "no pty_id"
LEASE=$(jq -r '.lease_token // empty' <<<"$SPAWN" 2>/dev/null)
[ -n "$LEASE" ] && ok "lease acquired at spawn" || bad "no lease_token"
[ "$(jq -r '.status // empty' <<<"$SPAWN" 2>/dev/null)" = running ] && ok "status=running" || bad "not running"
[ "$(jq -r '.label // empty' <<<"$SPAWN" 2>/dev/null)" = "$RUN-a" ] && ok "label applied" || bad "label missing"

hdr "3. spawned terminal is visible to terminal-list"
"$IYKE" --json terminals | jq -e --arg t "$TID" 'any(.[]; .terminal_id==$t)' >/dev/null \
  && ok "listed" || bad "not in terminal list"

hdr "4. duplicate label rejected"
# Guards against stranding an unnamed terminal the caller never asked for.
#
# KNOWN FLAKE — this is a true positive, do NOT weaken it to make it green.
# `post_terminal_spawn` rejects a duplicate only if some terminal holding that
# label is currently `status == "running"`. A single terminal_id can own more
# than one pty record, and there is a window where the outgoing record has
# flipped to `exited` while its replacement has not yet reached `running`
# (React StrictMode double-mounts in dev; in production the same shape appears
# whenever a shell exits and is restarted from the pane). Spawning inside that
# window sees no running holder and is allowed, so two terminals end up sharing
# a label and label addressing becomes ambiguous. Reproduced roughly 1 run in 5
# against a dev shell on 2026-07-25. Fix belongs shell-side, in the guard.
DUP=$("$IYKE" --json terminal-spawn --label "$RUN-a" --cwd /tmp -- bash --norc 2>&1)
if grep -qiE "already in use|conflict|409" <<<"$DUP"; then
  ok "rejected"
else
  # The race actually created a terminal. Track it so cleanup reclaims it
  # instead of leaving an orphan tab behind.
  STRAY=$(jq -r '.terminal_id // empty' <<<"$DUP" 2>/dev/null)
  [ -n "$STRAY" ] && SPAWNED+=("$STRAY")
  bad "duplicate label accepted — see the comment above: $DUP"
fi

hdr "5. leased write reaches the real pty"
MARKER="LIVE_MARKER_$RUN"
"$IYKE" terminal-send "echo $MARKER" --label "$RUN-a" --key Enter \
        --lease-token "$LEASE" --actor "$AGENT" >/dev/null 2>&1
FOUND=no
for _ in $(seq 1 10); do
  OUT=$("$IYKE" --json terminal-read --label "$RUN-a" --after 0 2>/dev/null | jq -r '.text // ""')
  if grep -q "$MARKER" <<<"$OUT"; then FOUND=yes; break; fi
  sleep 0.5
done
[ "$FOUND" = yes ] && ok "marker echoed in scrollback" || bad "marker never appeared"

hdr "6. task board round-trip"
TASK=$("$IYKE" --json task create "[live-test] control-plane check" --priority high \
        --assigned-to "$AGENT" --actor "$AGENT" 2>&1)
TASK_ID=$(jq -r '.id // empty' <<<"$TASK")
[ -n "$TASK_ID" ] && ok "created" || bad "create failed: $TASK"
"$IYKE" --json task list --status pending \
  | jq -e --arg i "$TASK_ID" '.tasks | any(.[]; .id==$i)' >/dev/null \
  && ok "appears in pending" || bad "not in pending list"
"$IYKE" task update "$TASK_ID" --status in_progress --progress-pct 40 >/dev/null 2>&1 \
  && ok "updated" || bad "update failed"
"$IYKE" task complete "$TASK_ID" --task-result "live-verified" >/dev/null 2>&1 \
  && ok "completed" || bad "complete failed"
"$IYKE" --json task list --status completed \
  | jq -e --arg i "$TASK_ID" '.tasks | any(.[]; .id==$i)' >/dev/null \
  && ok "moved to completed" || bad "not in completed list"

hdr "7. no-op task update rejected client-side"
"$IYKE" task update "$TASK_ID" >/dev/null 2>&1 && bad "no-op accepted" || ok "rejected"

hdr "8. timer against an unregistered agent is actionable"
# Without this the caller gets a bare `FOREIGN KEY constraint failed`.
PORT=$(jq -r .port "$CTL"); TOKEN=$(jq -r .token "$CTL")
UNREG=$(curl -s -X POST "http://127.0.0.1:$PORT/iyke/timer/schedule" \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"agent_id":"definitely-not-registered","delay_ms":1000,"title":"probe"}' 2>&1)
grep -qi "unknown agent_id" <<<"$UNREG" && ok "names the fix" || bad "unexpected: $UNREG"

hdr "9. timer delivers into the inbox (the WP-09 loop)"
# This is the path that was dead before WP-09: fire_due_timer always wrote to
# iyke_agent_inbox, but nothing could read it back out.
BEFORE=$("$IYKE" --json inbox list --agent-id "$AGENT" | jq -r '.next_since // 0')
curl -s -X POST "http://127.0.0.1:$PORT/iyke/timer/schedule" \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d "{\"agent_id\":\"$AGENT\",\"delay_ms\":1500,\"title\":\"live inbox probe $RUN\"}" >/dev/null 2>&1
DELIVERED=no
for _ in $(seq 1 20); do
  INBOX=$("$IYKE" --json inbox list --agent-id "$AGENT" --since "$BEFORE" 2>/dev/null)
  if jq -e --arg r "$RUN" '.entries | any(.[]; (.payload|tostring) | contains($r))' <<<"$INBOX" >/dev/null 2>&1; then
    DELIVERED=yes; break
  fi
  sleep 0.5
done
[ "$DELIVERED" = yes ] && ok "timer landed in inbox" || bad "timer never delivered"

hdr "10. ack deletes, cursor advances"
IDS=$(jq -r '.entries[].id' <<<"$INBOX" 2>/dev/null | sed 's/^/--id /' | tr '\n' ' ')
if [ -n "$IDS" ]; then
  # shellcheck disable=SC2086
  "$IYKE" inbox ack $IDS >/dev/null 2>&1 && ok "acked" || bad "ack failed"
  AFTER=$("$IYKE" --json inbox list --agent-id "$AGENT" --since "$BEFORE" 2>/dev/null)
  jq -e --arg r "$RUN" '.entries | any(.[]; (.payload|tostring) | contains($r)) | not' <<<"$AFTER" >/dev/null \
    && ok "entry gone after ack" || bad "entry survived ack"
else
  bad "no ids to ack"
fi

hdr "11. terminal-kill retains the tab"
KILL=$("$IYKE" --json terminal-kill "$RUN-a" 2>&1)
jq -e '.ok == true' <<<"$KILL" >/dev/null && ok "killed" || bad "kill failed: $KILL"
# Assert on the concrete pty_id, NOT the terminal_id. One terminal_id can own
# several pty records at once — React StrictMode double-mounts in dev, and in
# production a shell that exits and restarts from the pane leaves the old
# record behind for ~10min. Matching on terminal_id and taking the first hit
# reads an arbitrary record and flakes (seen: "expected exited, got running").
GONE=no
for _ in $(seq 1 10); do
  ST=$("$IYKE" --json terminals | jq -r --arg p "$PTY" '.[] | select(.pty_id==$p) | .status')
  if [ "$ST" = exited ]; then GONE=yes; break; fi
  sleep 0.5
done
[ "$GONE" = yes ] && ok "descriptor status=exited, tab retained" || bad "expected exited, got '${ST:-<gone>}'"

echo
echo "════════ PASS=$PASS FAIL=$FAIL ════════"
[ "$FAIL" -eq 0 ]
