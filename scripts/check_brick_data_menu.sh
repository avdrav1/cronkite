#!/usr/bin/env bash
# APP-10 acceptance: Brick's "First Row Is Header" and Delimiter are in the Data menu (Linux:
# the main menu's Data), not a button and a dropdown in the header bar.
#
#   scripts/check_brick_data_menu.sh [BIN_DIR]   (default target/release)
#
# In an Arch container under headless sway (1280x720), reading what Brick shows through the
# accessibility tree (scripts/update_ui_helpers.py), keys through wtype:
#   1. a CSV whose first row names its columns opens with no "Header row" button and no
#      delimiter dropdown, 3 rows;
#   2. F10, Data: "First Row Is Header" checked (screenshot);
#   3. Enter on it: the first row is data, 4 rows, and the item unchecked (screenshot);
#   4. Enter on it again: 3 rows, checked;
#   5. Data, Delimiter: Auto chosen; Semicolon: the status says Semicolon, Semicolon
#      chosen (screenshot); Comma: the status says Comma;
#   6. started again, with an edit unsaved, Delimiter, Tab: an alert says to save first,
#      and the status still says Comma (detected) (screenshot).
# Needs docker. VERBOSE=1 prints the transcript; SHOTS=DIR saves the screenshots.
set -euo pipefail
cd "$(dirname "$0")/.."
bin=$(realpath "${1:-target/release}")
[ -x "$bin/brick" ] || { echo "no $bin/brick (cargo build --release -p brick)"; exit 1; }
shots=()
if [ -n "${SHOTS:-}" ]; then mkdir -p "$SHOTS"; shots=(-v "$(realpath "$SHOTS"):/shots"); fi
# Arch's sway binary carries cap_sys_nice: exec fails without it in the container's set.
out=$(docker run --rm --cap-add SYS_NICE -v "$bin/brick:/usr/local/bin/brick:ro" \
  -v "$PWD/scripts:/scripts:ro" "${shots[@]}" archlinux:latest bash -c '
set -uo pipefail
pacman -Syu --noconfirm --needed sway wtype grim gtk4 ttf-dejavu dbus python-gobject at-spi2-core \
  >/dev/null 2>&1
mkdir -p /shots /tmp/rt && chmod 700 /tmp/rt
export XDG_RUNTIME_DIR=/tmp/rt GDK_DEBUG=no-portals GDK_BACKEND=wayland CRONKITE_NO_UPDATE=1
printf "output HEADLESS-1 resolution 1280x720 position 0 0\ndefault_border none\n" > /tmp/sway.conf
printf "id,name,city\n1,Ann,Oslo\n2,Bo,Rome\n3,Cy,Lima\n" > /tmp/people.csv
run() {
  WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 sway --unsupported-gpu -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
  for i in $(seq 50); do ls /tmp/rt/wayland-[0-9] >/dev/null 2>&1 && break; sleep 0.1; done
  export WAYLAND_DISPLAY=$(basename $(ls /tmp/rt/wayland-[0-9] | head -1))
  export SWAYSOCK=$(ls /tmp/rt/sway-ipc.*.sock | head -1)
  shot() { grim /tmp/shot.png 2>/dev/null; cp /tmp/shot.png /shots/brick-$1.png 2>/dev/null || true; }
  keys() { wtype -s 300 "$@" -s 400; }
  ui() { python3 /scripts/update_ui_helpers.py brick "$@"; }
  # The status bar says how many rows there are ("3 rows").
  rows() { ui text | grep -oE "^[0-9,]+ rows" | head -1; }
  # The main menu open, Data'\''s submenu open, First Row Is Header focused (after the two
  # sort items).
  data() { keys -k F10; sleep 1; keys -k Return; sleep 1; keys -k Down -k Down -k Down; sleep 0.5; }
  # Data, Delimiter'\''s submenu open (after First Row Is Header), its N-th item focused.
  delim() {
    data; keys -k Down -k Return; sleep 1
    for i in $(seq $1); do keys -k Down; done; sleep 0.5
  }
  # The status bar'\''s delimiter ("Comma (detected)", "Semicolon").
  sep() { ui text | grep -oE "^(Comma|Tab|Semicolon|Pipe)[^·]*" | head -1 | sed "s/ *$//"; }
  brick /tmp/people.csv >/tmp/brick.log 2>&1 &
  sleep 5
  swaymsg "[app_id=dev.cronkite.Brick] focus" >/dev/null; sleep 1

  echo "== 1 open"
  echo "button: $(ui item "Header row")"
  echo "dropdown: $(ui text | grep -cx Auto)"
  echo "rows: $(rows)"
  shot 1-open

  echo "== 2 menu"
  data
  echo "item: $(ui item "First Row Is Header")"
  shot 2-menu

  echo "== 3 off"
  keys -k Return; sleep 1.5
  echo "rows: $(rows)"
  data
  echo "item: $(ui item "First Row Is Header")"
  shot 3-off

  echo "== 4 on"
  keys -k Return; sleep 1.5
  echo "rows: $(rows)"
  data
  echo "item: $(ui item "First Row Is Header")"
  keys -k Escape -k Escape -k Escape; sleep 0.5

  echo "== 5 delimiter"
  delim 4
  echo "auto: $(ui item Auto)"
  keys -k Return; sleep 1.5
  echo "status: $(sep)"
  delim 4
  echo "semicolon: $(ui item Semicolon)"
  shot 5-semicolon
  keys -k Up -k Up -k Return; sleep 1.5
  echo "status: $(sep)"

  echo "== 6 unsaved"
  # A closed menu gives the keys back to the main menu button: start again for the grid.
  pkill -x brick; sleep 1
  brick /tmp/people.csv >/tmp/brick.log 2>&1 &
  sleep 5
  swaymsg "[app_id=dev.cronkite.Brick] focus" >/dev/null; sleep 1
  keys -k Down x -k Return; sleep 1
  delim 3
  keys -k Return; sleep 1.5
  ui text | grep -x "Save your edits before changing the delimiter" | sed "s/^/alert: /"
  shot 6-unsaved
  keys -k Return; sleep 1
  echo "status: $(sep)"
  pkill -x brick
}
dbus-run-session -- bash -c "$(declare -f run); run" 2>&1
' 2>&1) || true
[ -n "${VERBOSE:-}" ] && printf '%s\n' "$out"

fails=0 checks=0
check() { # SECTION LINE: the transcript's SECTION has LINE
  checks=$((checks + 1))
  if ! printf '%s\n' "$out" | awk -v s="== $1" '$0 ~ /^== / {on = ($0 == s); next} on' |
    grep -qxF -- "$2"; then
    echo "FAIL: $1: no \"$2\""; fails=$((fails + 1))
  fi
}
check "1 open" "button: none"
check "1 open" "rows: 3 rows"
check "2 menu" "item: checked"
check "3 off" "rows: 4 rows"
check "3 off" "item: unchecked"
check "4 on" "rows: 3 rows"
check "4 on" "item: checked"
check "1 open" "dropdown: 0"
check "5 delimiter" "auto: checked"
check "5 delimiter" "semicolon: checked"
check "5 delimiter" "status: Semicolon"
check "5 delimiter" "status: Comma"
check "6 unsaved" "alert: Save your edits before changing the delimiter"
check "6 unsaved" "status: Comma (detected)"
if [ $fails -gt 0 ]; then
  [ -z "${VERBOSE:-}" ] && printf '%s\n' "$out"
  echo "check_brick_data_menu: $fails of $checks checks failed"; exit 1
fi
echo "check_brick_data_menu: OK ($checks checks)"
