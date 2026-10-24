#!/usr/bin/env bash
# UPD-7 on Linux: an installed copy of a live release updates itself from the live
# latest.json, in the wild (ADR 0012). In a clean Arch container under headless sway, as a
# normal user, for Brick, Slate, Frame, and Etch:
#
#   scripts/check_update_live.sh
#
#   1. the app's real installer (https://<app>.cronkite.cc/install.sh) installs the live
#      release FROM: scripts/pin_release.py serves it a latest.json naming FROM's live files
#      (APP_DOWNLOAD_URL; the installer checks their sha256); `--version` is recorded;
#   2. the installed app (~/.local/bin/<app>, so it counts as an installer's install) starts
#      with no CRONKITE_UPDATE_URL and no test key: it checks https://dl.cronkite.cc's
#      latest.json, signed with a release key;
#   with EXPECT_UPDATE=1 (the default), the live latest must be newer than FROM, and:
#   3. while FROM runs it logs "update: <App> LATEST installed for the next start" and
#      "... downloaded for the next start" (screenshot), then quits;
#   4. `--version` is LATEST (recorded), updates/applied names it, mimeapps.list is as the
#      installer left it;
#   5. the next start logs "update: Updated to <App> LATEST" and its window shows the notice
#      (AT-SPI; screenshot of the banner); mimeapps.list still unchanged;
#   with EXPECT_UPDATE=0 the live latest must be FROM itself (the release before the next
#   one is out), and:
#   3. the check runs to its end (updates/.lock taken, then released) without a problem
#      logged (latest.json and its signature verified) and stages nothing: up to date
#      (screenshot); after quitting, `--version` is still FROM and mimeapps.list unchanged.
#
# FROM per app: FROM_BRICK (default 0.6.0), FROM_SLATE (0.6.0), FROM_FRAME (0.4.0),
# FROM_ETCH (0.5.0): the first releases carrying the updater. APPS="etch brick" checks some
# apps. SHOTS=DIR keeps the screenshots, each app's logs, and summary.txt (the recorded
# versions and results). VERBOSE=1 prints the container's transcript. Needs docker, python3,
# curl, and the network. Prints `check_update_live: OK (N checks)` when all pass.
set -euo pipefail
cd "$(dirname "$0")/.."
APPS=${APPS:-brick slate frame etch}
EXPECT_UPDATE=${EXPECT_UPDATE:-1}
case $EXPECT_UPDATE in 0 | 1) ;; *) echo "EXPECT_UPDATE is 0 or 1, not $EXPECT_UPDATE"; exit 2 ;; esac
declare -A FROM=([brick]=${FROM_BRICK:-0.6.0} [slate]=${FROM_SLATE:-0.6.0} [frame]=${FROM_FRAME:-0.4.0} [etch]=${FROM_ETCH:-0.5.0})
work=target/check-update-live
site=$work/site
rm -rf "$site" "$work/shots" "$work/linux.out"
mkdir -p "$site" "$work/shots"

# What the apps check: each one's live latest.json (suite-core's AppKind::release_url).
live() { if [ "$1" = brick ]; then echo https://dl.cronkite.cc; else echo "https://dl.cronkite.cc/$1"; fi; }
declare -A LATEST
fail=0 run=""
pre=$(for a in $APPS; do
  case $a in brick | slate | frame | etch) ;; *) echo "FAIL unknown app $a"; continue ;; esac
  latest=$(curl -fsS "$(live "$a")/latest.json" | python3 -c 'import json, sys; print(json.load(sys.stdin)["version"])') ||
    { echo "FAIL $a: no live latest.json at $(live "$a")"; continue; }
  echo "latest $a $latest"
  if [ "$EXPECT_UPDATE" = 1 ] && [ "$latest" = "${FROM[$a]}" ]; then
    echo "FAIL $a: no newer release live: $(live "$a")/latest.json is ${FROM[$a]} (EXPECT_UPDATE=0 checks the up-to-date path)"
  elif [ "$EXPECT_UPDATE" = 0 ] && [ "$latest" != "${FROM[$a]}" ]; then
    echo "FAIL $a: $(live "$a")/latest.json is $latest, not ${FROM[$a]}: EXPECT_UPDATE=0 expects FROM to be the live latest"
  else
    echo "run $a"
  fi
done)
grep '^FAIL' <<<"$pre" && fail=1
while read -r _ a v; do LATEST[$a]=$v; done < <(grep '^latest ' <<<"$pre")
run=$(sed -n 's/^run //p' <<<"$pre" | paste -sd' ')

if [ -n "$run" ]; then
  specs=""; for a in $run; do specs="$specs $a:${FROM[$a]}"; done
  # shellcheck disable=SC2086
  python3 scripts/pin_release.py "$site" $specs
  {
    echo "APPS='$run' EXPECT=$EXPECT_UPDATE"
    for a in $run; do echo "FROM_$a=${FROM[$a]} LATEST_$a=${LATEST[$a]}"; done
  } >"$site/versions"
  cp scripts/update_ui_helpers.py "$site/"
  cat >"$site/run.sh" <<'EOF'
#!/bin/bash
# check_update_live.sh's steps as the user tester, in a D-Bus session; prints ok/FAIL lines
# and "note" lines with what it recorded.
set -u
. /site/versions
cd "$HOME"
mkdir -p shots logs ~/.config
export XDG_RUNTIME_DIR=/tmp/rt-tester GDK_DEBUG=no-portals GDK_BACKEND=wayland
mkdir -p -m 700 "$XDG_RUNTIME_DIR"
# The pinned site, for the installers only.
python3 -m http.server 8765 --bind 127.0.0.1 --directory /site >/dev/null 2>&1 &
printf 'output HEADLESS-1 resolution 1280x720 position 0 0\ndefault_border none\n' >/tmp/sway.conf
WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 sway --unsupported-gpu -c /tmp/sway.conf >logs/sway.log 2>&1 &
for i in $(seq 50); do ls "$XDG_RUNTIME_DIR"/wayland-[0-9] >/dev/null 2>&1 && break; sleep 0.1; done
WAYLAND_DISPLAY=$(basename "$(ls "$XDG_RUNTIME_DIR"/wayland-[0-9] | head -1)")
SWAYSOCK=$(ls "$XDG_RUNTIME_DIR"/sway-ipc.*.sock | head -1)
export WAYLAND_DISPLAY SWAYSOCK
for i in $(seq 50); do curl -fsS -o /dev/null http://127.0.0.1:8765/versions && break; sleep 0.1; done
# Another app's default beside the ones install.sh sets.
printf '[Default Applications]\ntext/plain=gedit.desktop\n' >~/.config/mimeapps.list
# A 64x32 PNG for Etch to open.
python3 - <<"PY"
import struct, zlib
w, h = 64, 32
raw = b"".join(b"\0" + bytes([200, 40, 40]) * w for _ in range(h))
def chunk(t, d):
    return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
png += chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")
open("/tmp/red.png", "wb").write(png)
PY
BIN=$HOME/.local/bin

check() { # DESCRIPTION COMMAND...: one ok or FAIL line; a failure shows the current log
  d=$1; shift
  if "$@" >/dev/null 2>&1; then echo "ok   $app: $d"
  else echo "FAIL $app: $d"; [ -f "${log:-}" ] && tail -n 15 "$log" | sed 's/^/     | /'; fi
}
note() { echo "note $app: $*"; }
title() { case $1 in brick) echo Brick ;; slate) echo Slate ;; frame) echo Frame ;; etch) echo Etch ;; esac; }
said() { printf '%s\n' "$out" | grep -q -- "$1"; }
version_is() { [ "$("$BIN/$app" --version 2>/dev/null)" = "$app $1" ]; }
same() { [ "$1" = "$2" ]; }
mimeapps() { sha256sum ~/.config/mimeapps.list; }
# The installed app (Etch on a file), its window focused; sets $pid and $log.
start() {
  log=$HOME/logs/$app-$1.log
  if [ "$app" = etch ]; then "$BIN/$app" /tmp/red.png >"$log" 2>&1 & else "$BIN/$app" >"$log" 2>&1 & fi
  pid=$!
  sleep 5
  swaymsg "[app_id=dev.cronkite.$App] focus" >/dev/null 2>&1; sleep 1
}
running() { kill -0 "$pid"; }
quit() { kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null; }
saw() { # PATTERN [SECONDS]: in the log while the app runs
  i=0
  while [ $i -lt $((${2:-300} * 2)) ]; do
    grep -q -- "$1" "$log" && return 0
    kill -0 "$pid" 2>/dev/null || return 1
    sleep 0.5; i=$((i + 1))
  done
  return 1
}
# The check ran to its end: it takes updates/.lock (made then) for as long as it runs.
finished() {
  i=0
  while [ ! -e "$updates/.lock" ]; do i=$((i + 1)); [ $i -lt 120 ] || return 1; sleep 0.5; done
  sleep 2
  i=0
  until flock -n "$updates/.lock" true; do i=$((i + 1)); [ $i -lt 240 ] || return 1; sleep 0.5; done
}
no_update_lines() { ! grep -q "update:" "$log"; }
# Nothing in updates/ but the check's .lock and the running instances' owner-PID.lock.
nothing_staged() { [ -z "$(ls "$updates" 2>/dev/null | grep -v '^owner-[0-9]*\.lock$')" ]; }
applied_is() { grep -qx "version=$1" "$updates/applied"; }
notice_shown() { python3 /site/update_ui_helpers.py "$app" text | grep -qx "Updated to $App $1"; }
shot() { grim "$HOME/shots/$app-$1.png"; }

for app in $APPS; do
  eval "from=\$FROM_$app latest=\$LATEST_$app"
  App=$(title "$app") log=""
  updates=$HOME/.cache/$app/updates
  rm -rf "$HOME/.cache/$app"
  up=$(echo "$app" | tr a-z A-Z)
  out=$(curl -fsSL "http://127.0.0.1:8765/$app/install.sh" | env "${up}_DOWNLOAD_URL=http://127.0.0.1:8765/$app" sh 2>&1)
  printf '%s\n' "$out" >"logs/$app-install.log"
  check "https://$app.cronkite.cc/install.sh installs the live $App $from" said "Installed $App $from"
  note "--version before: $("$BIN/$app" --version 2>&1)"
  check "--version is $from" version_is "$from"
  before=$(mimeapps)

  start "1-$from"
  check "$from starts" running
  if [ "$EXPECT" = 1 ]; then
    check "the running $from installs the live $latest ('installed for the next start')" saw "update: $App $latest installed for the next start"
    check "the check reports $latest downloaded from the live latest.json" saw "update: $App $latest downloaded for the next start"
    shot "1-$from-updating"
    note "logged: $(grep -o 'update: .*' "$log" | paste -sd'|')"
    check "$from keeps running after the update" running
    quit
    note "--version after: $("$BIN/$app" --version 2>&1)"
    check "after quitting, --version is $latest" version_is "$latest"
    check "updates/applied names $latest" applied_is "$latest"
    check "mimeapps.list unchanged by the update" same "$before" "$(mimeapps)"

    start "2-$latest"
    check "the next start shows 'Updated to $App $latest' (stderr)" saw "update: Updated to $App $latest" 60
    sleep 1
    check "the window shows the notice 'Updated to $App $latest'" notice_shown "$latest"
    shot "2-$latest-updated"
    note "logged: $(grep -o 'update: .*' "$log" | paste -sd'|')"
    check "the notice's record is consumed" test ! -e "$updates/applied"
    quit
    check "mimeapps.list unchanged after the next start" same "$before" "$(mimeapps)"
  else
    check "the check of the live latest.json runs to its end (updates/.lock taken, released)" finished
    check "it logs no problem: latest.json and its signature verified, nothing newer" no_update_lines
    check "nothing staged: $from is the live latest" nothing_staged
    shot "1-$from-up-to-date"
    note "up to date: $App $from is the live latest; updates/: $(ls -A "$updates" | paste -sd' ')"
    check "$from keeps running" running
    quit
    note "--version after: $("$BIN/$app" --version 2>&1)"
    check "after quitting, --version is still $from" version_is "$from"
    check "mimeapps.list unchanged" same "$before" "$(mimeapps)"
  fi
done
swaymsg exit >/dev/null 2>&1
echo "run.sh: finished"
EOF
  chmod -R a+rX "$site"
  # Arch's sway binary carries cap_sys_nice: exec fails without it in the container's set.
  docker run --rm --cap-add SYS_NICE -v "$PWD/$site:/site:ro" -v "$PWD/$work/shots:/out" archlinux:latest bash -c '
    set -e
    pacman -Syu --noconfirm --needed gtk4 sway grim ttf-dejavu dbus python python-gobject at-spi2-core curl util-linux >/dev/null 2>&1
    useradd -m tester
    status=0
    su tester -c "dbus-run-session -- bash /site/run.sh" || status=$?
    cp -r /home/tester/shots/. /home/tester/logs /out/ 2>/dev/null || true
    chmod -R a+rwX /out
    exit $status
  ' >"$work/linux.out" 2>&1 || true
  [ -n "${VERBOSE:-}" ] && sed 's/^/  /' "$work/linux.out"
  grep -q '^run.sh: finished$' "$work/linux.out" || { echo "FAIL the container didn't finish:"; tail -n 30 "$work/linux.out" | sed 's/^/     /'; fail=1; }
  grep -E '^(ok|FAIL|note) |^     \|' "$work/linux.out" || true
  grep -q '^FAIL ' "$work/linux.out" && fail=1
fi

total=$(grep -c '^ok ' "$work/linux.out" 2>/dev/null || true)
summary=$( {
  echo "UPD-7 in the wild, Linux (Arch, headless sway), $(date -u +%Y-%m-%dT%H:%MZ), EXPECT_UPDATE=$EXPECT_UPDATE"
  for a in $APPS; do echo "$a: FROM ${FROM[$a]}, live latest ${LATEST[$a]:-?}"; done
  printf '%s\n' "$pre" | grep '^FAIL' || true
  grep -E '^(ok|FAIL|note) ' "$work/linux.out" 2>/dev/null || true
})
if [ -n "${SHOTS:-}" ]; then
  mkdir -p "$SHOTS"
  cp -r "$work/shots/." "$SHOTS/"
  printf '%s\n' "$summary" >"$SHOTS/summary.txt"
fi
if [ $fail = 0 ] && [ "${total:-0}" -gt 0 ]; then echo "check_update_live: OK ($total checks)"; else
  echo "check_update_live: FAILED"
  [ -n "${VERBOSE:-}" ] || [ ! -f "$work/linux.out" ] || echo "(VERBOSE=1 for the transcript; $work/linux.out)"
  exit 1
fi
