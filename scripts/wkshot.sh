#!/bin/bash
# The desktop window's engine (WebKitGTK 2.52 MiniBrowser) in Xvfb at a given size: load URL, wait, run xdotool steps, shoot.
# usage: wkshot.sh WxH URL OUT.png [WAIT_S] [-- xdotool steps separated by ';']
S=${STELLA_WK_DIR:-${TMPDIR:-/tmp}/stella-wk}; mkdir -p "$S/home"
SIZE=$1 URL=$2 OUT=$3 WAIT=${4:-10}; shift 4; STEPS="$*"
export XAUTHORITY=/dev/null DISPLAY=:98
Xvfb :98 -ac -screen 0 ${SIZE}x24 >/dev/null 2>&1 & XP=$!; sleep 1.5
HOME=$S/home /usr/lib/x86_64-linux-gnu/webkit2gtk-4.1/MiniBrowser "$URL" >/dev/null 2>&1 & MP=$!
sleep 2; W=$(xdotool search --pid $MP 2>/dev/null | tail -1); for w in $(xdotool search --pid $MP 2>/dev/null); do xdotool windowsize $w ${SIZE%x*} ${SIZE#*x} windowmove $w 0 0 2>/dev/null; done
sleep "$WAIT"
IFS=';' read -ra A <<< "$STEPS"; for s in "${A[@]}"; do [ -n "$s" ] && { xdotool $s; sleep 1.2; }; done
import -window root "$OUT"
kill $MP; sleep 0.5; kill $XP
