#!/usr/bin/env bash
# Stella Scope desktop launcher — the tracked version (2026-09-18).
# Starts the recorder service and opens the Cockpit window in Brave. When the window
# closes, a detached watcher keeps the recorder alive while a print runs and for
# GRACE_S seconds after it ends, so closing the window never stops a live recording.
# Laws: the watcher queries the binary's own read-only API — no python3, no printer
# commands. If the watcher dies, the failure direction is safe: recording continues.
set -u
URL="${STELLA_SCOPE_URL:-http://127.0.0.1:7420/}"
GRACE_S="${STELLA_SCOPE_GRACE_S:-900}"
BROWSER="${STELLA_SCOPE_BROWSER:-/usr/bin/brave-browser}"

# Extract job state from /api/tick. Key order in the payload is alphabetical
# (duration, filament_used, name, progress, state), so the job object's "state" is
# matched inside its own braces. A parse failure returns nothing → the caller treats
# it as "unknown", which keeps the watcher waiting (never stops a live recording).
parse_state() { sed -n 's/.*"job":{[^{}]*"state":"\([a-z_]*\)".*/\1/p' | head -n 1; }

state() {
  local out
  out=$(curl -s --max-time 2 "${URL}api/tick" 2>/dev/null | parse_state) || true
  if [ -n "$out" ]; then echo "$out"; return; fi
  # Empty parse: distinguish "service down" from "payload without a job state".
  if curl -s -o /dev/null --max-time 2 "${URL}api/meta" 2>/dev/null; then echo unknown; else echo down; fi
}

# Stella's window is found by its WM_CLASS, never by title (2026-09-26: a test browser titled "Stella
# Scope" and other 127.0.0.1 app windows were taken for it). `wmctrl -lx` prints instance.class.
stella_window() { wmctrl -lx 2>/dev/null | awk '$3 ~ /\.(StellaScope|Stella-desktop)$/ {print $1; exit}'; }
raise_stella() { local id; id=$(stella_window); [ -n "$id" ] && wmctrl -i -a "$id" 2>/dev/null; }

if [ "${1:-}" = "--self-test" ]; then
  fails=0
  expect() { # description payload expected
    local got
    got=$(printf '%s' "$2" | parse_state); [ -z "$got" ] && got=unknown
    if [ "$got" != "$3" ]; then echo "FAIL: $1 (got '$got', want '$3')"; fails=$((fails+1)); fi
  }
  expect "alphabetical job keys" '{"job":{"duration":9.5,"filament_used":123.4,"name":"cube.gcode","progress":0.42,"state":"printing"}}' printing
  expect "paused state" '{"job":{"duration":0,"filament_used":0,"name":"x.gcode","progress":0,"state":"paused"}}' paused
  expect "standby state" '{"job":{"duration":0,"filament_used":0,"name":"x.gcode","progress":0,"state":"standby"}}' standby
  expect "null job is unknown (keep recording)" '{"schema":"stella-scope.tick.v1","job":null}' unknown
  expect "no state key is unknown" '{"job":{"name":"x.gcode"}}' unknown
  expect "brace filename stays unknown (safe direction)" '{"job":{"duration":0,"name":"a{b}.gcode","state":"complete"}}' unknown
  expect "garbage is unknown" 'not json at all' unknown
  if [ "$fails" -gt 0 ]; then echo "launcher self-test: $fails FAILED"; exit 1; fi
  echo "launcher self-test: 7/7 ok"; exit 0
fi

# Single instance: a second icon raises the existing window instead.
#
# 2026-09-21: the app stopped opening at all, with no window and no taskbar dot. The startup
# lock was taken with `exec 9>`, so the detached watcher inherited fd 9 and held the lock for the
# whole grace period after the window closed -- up to 15 minutes. Every launch in that window hit
# `flock -n`, failed, found no window to raise, and exited silently. Two changes keep that from
# coming back: the lock now covers only this check-and-open (nothing that outlives the launcher is
# allowed to inherit fd 9, see the `9>&-` on the browser and watcher below), and a real launch that
# collides with another one waits its turn instead of giving up.
exec 9>"${XDG_RUNTIME_DIR:-/tmp}/stella-scope.lock"
if ! flock -w 10 9; then
  raise_stella
  exit 0
fi
# Window already up: raise it, do not open a second one.
if [ -n "$(stella_window)" ]; then
  raise_stella
  exit 0
fi

systemctl --user start stella-scope.service

# Give the service a moment; the window opens either way.
ok=0
for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30; do
  if curl -s -o /dev/null --max-time 1 "${URL}api/meta" 2>/dev/null; then ok=1; break; fi
  sleep 1
done
[ "$ok" = 0 ] && notify-send -u critical "Stella Scope" "Recorder service did not answer on ${URL} — check: systemctl --user status stella-scope.service"

# 9>&- so the browser never carries the startup lock: Brave outlives this script.
# 2026-09-26: the Stella Scope icon opened the new 3D3D Studio. Every Brave --app window on
# 127.0.0.1 is named "127.0.0.1" to the desktop (the running Brave drops --class), so the icon
# claimed whichever loopback app was open. Stella now opens in its own small Brave profile, a
# separate Brave process that keeps --class, so the window is "StellaScope" and nothing else is.
# 2026-09-26 (the owner: "no longer running just in my browser"): when the Stella Scope desktop app is
# installed (the .deb puts it at /usr/bin/stella-desktop), the cockpit opens in its own window
# (WM_CLASS stella-desktop.Stella-desktop). It finds this running service and opens it as it is.
DESKTOP_APP="${STELLA_SCOPE_APP:-/usr/bin/stella-desktop}"
if [ -x "$DESKTOP_APP" ]; then
  "$DESKTOP_APP" >/dev/null 2>&1 9>&- &
  disown
else
PROFILE="${STELLA_SCOPE_PROFILE:-$HOME/.local/share/stella-scope/brave-profile}"
mkdir -p "$PROFILE"
"$BROWSER" --user-data-dir="$PROFILE" --class=StellaScope --no-first-run --no-default-browser-check --app="${URL}" >/dev/null 2>&1 9>&- &
disown
fi

# Watcher: always detached (closes the review gap — the recorder now survives window
# close during a print AND for the grace period after one ends, not only when a print
# happens to be active at close time).
watch() {
  # The watcher's own lock, on its own file: it means "a watcher is alive", which is a different
  # question from "a launch is in progress". Keeping them apart is what lets the icon work while a
  # watcher is still counting down the grace period.
  exec 8>"${XDG_RUNTIME_DIR:-/tmp}/stella-scope-watch.lock"
  flock -n 8 || exit 0
  local idle=0 down=0
  while sleep 60; do
    # An open Cockpit window pins the recording, whatever the printer is doing.
    if [ -n "$(stella_window)" ]; then idle=0; down=0; continue; fi
    case "$(state)" in
      printing|paused|unknown) idle=0; down=0 ;;   # unknown = untrusted data; fail safe, keep recording
      down)
        down=$((down+1))
        if [ "$down" -ge 5 ]; then systemctl --user stop stella-scope.service; exit 0; fi
        ;;                                           # not answering for 5 min = tidy up
      *) idle=$((idle+60)); [ "$idle" -ge "$GRACE_S" ] && break ;;
    esac
  done
  systemctl --user stop stella-scope.service
}
watch >/dev/null 2>&1 9>&- &
disown
if command -v notify-send >/dev/null 2>&1; then
  case "$(state)" in
    printing|paused) notify-send "Stella Scope" "Window closed — recording continues while the print runs (up to ${GRACE_S}s after it ends)." ;;
  esac
fi
exit 0
