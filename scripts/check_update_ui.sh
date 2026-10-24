#!/usr/bin/env bash
# UPD-3 acceptance on Linux: "Update Automatically" and the "Updated to <App> X.Y.Z" notice
# in Brick, Slate, Frame, and Etch.
#
#   scripts/check_update_ui.sh [BIN_DIR]   (default target/release: brick, slate, frame, etch)
#
# In an Arch container under headless sway (1280x720). What each app shows is read through
# the accessibility tree (scripts/update_ui_helpers.py); keys go through wtype. For each app:
#   1. with an `applied` record naming the app's own version in its cache folder's
#      updates/ (what an update's apply step leaves, UPD-4 to UPD-6), the first start shows
#      "Updated to <App> X.Y.Z" in its window (screenshot), logs
#      "update: Updated to <App> X.Y.Z" on stderr, and the record is gone; Etch is started
#      on a file, so the notice survives "Opening…" giving way to the image;
#   2. the keys stay where they were: Return still opens Open (Brick's and Slate's start
#      windows), typing goes into the new deck's title (Frame), B picks the brush (Etch),
#      and the notice is still there;
#   3. Dismiss takes it away (Slate, Frame, Etch); Brick's goes by itself after 20 s;
#   4. the next start shows no notice and logs none (screenshot);
#   5. the main menu (F10; Etch: its menu bar's Help) has "Update Automatically", checked
#      (screenshot); Enter on it writes automatic=false in the settings folder's `update`;
#   6. the next start shows it unchecked (screenshot), and Enter turns it on again
#      (automatic=true).
# Needs docker. VERBOSE=1 prints the transcript; SHOTS=DIR saves the screenshots.
set -euo pipefail
cd "$(dirname "$0")/.."
bin=$(realpath "${1:-target/release}")
for app in brick slate frame etch; do
  [ -x "$bin/$app" ] || { echo "no $bin/$app (cargo build --release -p $app)"; exit 1; }
done
shots=()
if [ -n "${SHOTS:-}" ]; then mkdir -p "$SHOTS"; shots=(-v "$(realpath "$SHOTS"):/shots"); fi
# Arch's sway binary carries cap_sys_nice: exec fails without it in the container's set.
out=$(docker run --rm --cap-add SYS_NICE -v "$bin/brick:/usr/local/bin/brick:ro" \
  -v "$bin/slate:/usr/local/bin/slate:ro" -v "$bin/frame:/usr/local/bin/frame:ro" \
  -v "$bin/etch:/usr/local/bin/etch:ro" -v "$PWD/scripts:/scripts:ro" "${shots[@]}" \
  archlinux:latest bash -c '
set -uo pipefail
pacman -Syu --noconfirm --needed sway wtype grim gtk4 ttf-dejavu dbus python-gobject at-spi2-core \
  >/dev/null 2>&1
mkdir -p /shots /tmp/rt && chmod 700 /tmp/rt
export XDG_RUNTIME_DIR=/tmp/rt GDK_DEBUG=no-portals GDK_BACKEND=wayland
printf "output HEADLESS-1 resolution 1280x720 position 0 0\ndefault_border none\n" > /tmp/sway.conf
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
session() {
  WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 sway --unsupported-gpu -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
  for i in $(seq 50); do ls /tmp/rt/wayland-[0-9] >/dev/null 2>&1 && break; sleep 0.1; done
  export WAYLAND_DISPLAY=$(basename $(ls /tmp/rt/wayland-[0-9] | head -1))
  export SWAYSOCK=$(ls /tmp/rt/sway-ipc.*.sock | head -1)
  shot() { grim /tmp/shot.png 2>/dev/null; cp /tmp/shot.png /shots/$app-$1.png 2>/dev/null || true; }
  keys() { wtype -s 300 "$@" -s 400; }
  ui() { python3 /scripts/update_ui_helpers.py $app "$@"; }
  notice() { ui text | grep -c "^Updated to " || true; }
  # Start the app (Etch on a file), its window focused; its stderr in /tmp/$app.N.log.
  run=0
  start() {
    run=$((run + 1))
    if [ $app = etch ]; then etch /tmp/red.png >/tmp/$app.$run.log 2>&1 & else $app >/tmp/$app.$run.log 2>&1 & fi
    sleep 5
    swaymsg "[app_id=dev.cronkite.$name] focus" >/dev/null; sleep 1
  }
  stop() { pkill -x $app; sleep 1; pkill -9 -x $app; sleep 0.5; }
  # The main menu open, Update Automatically focused: it is the first item of Brick'\''s and
  # Frame'\''s; Slate'\''s second to last (Up from the first wraps to Quit); Help'\''s first in
  # Etch'\''s menu bar (F10, Down opens File, Left moves to Help).
  menu() {
    case $app in
      brick|frame) keys -k F10 ;;
      slate) keys -k F10 -k Up -k Up -k Up ;;
      etch) keys -k F10 -k Down -k Left ;;
    esac
    sleep 1
  }
  for spec in brick:Brick slate:Slate frame:Frame etch:Etch; do
    app=${spec%%:*}; name=${spec#*:}; run=0
    version=$($app --version | cut -d" " -f2)
    settings=$HOME/.config/$app/update; updates=$HOME/.cache/$app/updates
    rm -rf $HOME/.config/$app $HOME/.cache/$app $HOME/.local/state/$app
    mkdir -p $updates && echo "version=$version" > $updates/applied

    echo "== $app 1 notice"
    echo "version: $version"
    start
    ui text | grep "^Updated to " | sed "s/^/notice: /"
    shot 1-notice
    grep "^update: " /tmp/$app.$run.log | sed "s/^/log: /"
    [ -e $updates/applied ] && echo "record: kept" || echo "record: gone"

    echo "== $app 2 keys"
    case $app in
      brick|slate)
        keys -k Return; sleep 1.5
        echo "windows: $(ui windows | paste -sd, -)"
        keys -k Escape; sleep 1 ;;
      frame)
        keys Hi; sleep 1
        ui text | grep -q "Hi" && echo "typed: yes" || echo "typed: no" ;;
      etch)
        keys b; sleep 0.5
        echo "brush: $(ui item Brush)" ;;
    esac
    echo "notices: $(notice)"
    shot 2-keys

    echo "== $app 3 gone"
    if [ $app = brick ]; then sleep 18; else ui press Dismiss; sleep 1; fi
    echo "notices: $(notice)"
    stop

    echo "== $app 4 again"
    start
    echo "notices: $(notice)"
    echo "logged: $(grep -c "^update: " /tmp/$app.$run.log)"
    shot 4-again

    echo "== $app 5 menu"
    menu
    echo "item: $(ui item "Update Automatically")"
    shot 5-menu
    keys -k Return; sleep 1
    echo "settings: $(cat $settings 2>/dev/null)"
    stop

    echo "== $app 6 restart"
    start
    menu
    echo "item: $(ui item "Update Automatically")"
    shot 6-menu-off
    keys -k Return; sleep 1
    echo "settings: $(cat $settings 2>/dev/null)"
    stop
  done
  swaymsg exit >/dev/null 2>&1 || true
}
dbus-run-session -- bash -c "$(declare -f session); session" 2>/tmp/dbus.log
echo "== app log"; cat /tmp/*.log | grep -E "panicked|^thread|CRITICAL|^(brick|slate|frame|etch):" -A2 | head -30
' 2>&1) || true
[ -n "${VERBOSE:-}" ] && sed 's/^/  /' <<<"$out"
section() { awk -v s="== $1" '/^== /{on=($0 ~ "^" s)} on' <<<"$out"; }
fail=0
check() { # WHAT SECTION PATTERN
  if section "$2" | grep -E -- "$3" >/dev/null; then echo "ok   $1"; else echo "FAIL $1 (no match for: $3)"; fail=1; fi
}
absent() { # WHAT SECTION PATTERN
  if section "$2" | grep -E -- "$3" >/dev/null; then echo "FAIL $1 (found: $3)"; fail=1; else echo "ok   $1"; fi
}
for spec in brick:Brick slate:Slate frame:Frame etch:Etch; do
  app=${spec%%:*}; name=${spec#*:}
  v=$(section "$app 1" | sed -n "s/^version: //p")
  [ -n "$v" ] || { echo "FAIL $app: no version (did it run?)"; fail=1; continue; }
  check "$app 1: the first start after an update shows “Updated to $name ${v}”" "$app 1" "^notice: Updated to $name ${v//./\\.}$"
  check "$app 1: ... logs it on stderr" "$app 1" "^log: update: Updated to $name ${v//./\\.}$"
  check "$app 1: ... and the record of the update is gone" "$app 1" "^record: gone$"
  case $app in
    brick|slate) check "$app 2: the keys stay with the window: Return opens Open" "$app 2" "^windows: .*Open" ;;
    frame) check "$app 2: the keys stay with the window: typing goes into the title" "$app 2" "^typed: yes$" ;;
    etch) check "$app 2: the keys stay with the window: B picks the brush" "$app 2" "^brush: checked$" ;;
  esac
  check "$app 2: ... and the notice stays" "$app 2" "^notices: 1$"
  if [ $app = brick ]; then
    check "$app 3: the notice goes by itself after 20 s" "$app 3" "^notices: 0$"
  else
    check "$app 3: Dismiss takes the notice away" "$app 3" "^notices: 0$"
  fi
  check "$app 4: the next start shows no notice" "$app 4" "^notices: 0$"
  check "$app 4: ... and logs none" "$app 4" "^logged: 0$"
  check "$app 5: Update Automatically is in the menu, checked by default" "$app 5" "^item: checked$"
  check "$app 5: turning it off keeps automatic=false in the settings folder" "$app 5" "^settings: automatic=false$"
  check "$app 6: the next start shows it unchecked" "$app 6" "^item: unchecked$"
  check "$app 6: ... and turning it on keeps automatic=true" "$app 6" "^settings: automatic=true$"
done
absent "the apps log no errors or panics" "app log" "panicked|CRITICAL|^(brick|slate|frame|etch):"
if [ $fail = 0 ]; then echo "UPDATE UI OK"; else
  echo "UPDATE UI CHECK FAILED"; [ -n "${VERBOSE:-}" ] || echo "(VERBOSE=1 for the transcript)"
  exit 1
fi
