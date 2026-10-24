#!/usr/bin/env bash
# MAC-4 acceptance on macOS: each app's menus are in the menu bar and its windows have the
# system's title bar.
#
#   scripts/check_mac_ui.sh [BIN_DIR]   (default target/release; macOS only)
#
# Runs on GitHub's macOS runner (ci.yml's macos job), as check_mac_keys.sh does. Each app
# opens a sample file; through System Events (UI scripting) the check reads:
#   - the window's title bar buttons: macOS's own (AXCloseButton), not GNOME's drawn ones;
#   - the menu bar: the app menu, the app's menus (File, Edit, ...), and Window;
#   - the app menu's About and Quit, Quit with ⌘Q;
#   - UPD-3: "Update Automatically" right after About, checked (AXMenuItemMarkChar); a
#     click unchecks it and writes automatic=false in the settings folder's `update`; the
#     next start shows it unchecked, and a click turns it back on;
#   - UPD-3: with an `applied` record naming this version in the cache folder's updates/,
#     the first start logs "update: Updated to <App> X.Y.Z" (the window screenshot shows
#     the notice) and the next start doesn't;
#   - the File menu's and another menu's items with their keys (⌘S, ⇧⌘S, ...);
#   - the Window menu's Minimize and Zoom;
# then clicks the app menu, File, and another menu open for screenshots, opens About (a
# window more), and quits with ⌘Q (the process ends).
# Screenshots land in $OUT/shots (default out/mac-ui), item lists in $OUT/logs; a failure
# exits 1 after every app has run.
set -uo pipefail
bin=$(cd "${1:-target/release}" && pwd)
out=${OUT:-out/mac-ui}
mkdir -p "$out/shots" "$out/files" "$out/logs"
out=$(cd "$out" && pwd)
shots=$out/shots files=$out/files logs=$out/logs
root=$(cd "$(dirname "$0")/.." && pwd)
# shellcheck source=scripts/mac_helpers.sh
source "$root/scripts/mac_helpers.sh"

# System Events, told about process $pid.
proc() { osa "tell application \"System Events\" to tell (first process whose unix id is $pid) to $1" "${2:-}"; }

# The items of menu $2 of the menu bar into $logs/$app-$2.txt, a line each: name, key
# (AXMenuItemCmdChar), modifiers (AXMenuItemCmdModifiers: 0 ⌘, 1 ⇧⌘, 2 ⌥⌘, +4 ⌃, +8 no ⌘),
# separated by tabs.
menu_items() {
    local app=$1 menu=$2
    osa "tell application \"System Events\" to tell (first process whose unix id is $pid)
  set out to \"\"
  repeat with m in menu items of menu 1 of menu bar item \"$menu\" of menu bar 1
    set n to name of m
    if n is not missing value then
      set c to value of attribute \"AXMenuItemCmdChar\" of m
      set k to value of attribute \"AXMenuItemCmdModifiers\" of m
      set out to out & n & tab & (c as text) & tab & (k as text) & linefeed
    end if
  end repeat
  return out
end tell" "$logs/$app-$menu.txt"
}

# Item $3 of menu $2 shows key $4 with modifiers $5.
expect_key() {
    local app=$1 menu=$2 item=$3 char=$4 mods=$5 line
    line=$(awk -F'\t' -v n="$item" '$1 == n' "$logs/$app-$menu.txt" | head -1)
    local got_char got_mods
    got_char=$(cut -f2 <<<"$line" | tr '[:lower:]' '[:upper:]')
    got_mods=$(cut -f3 <<<"$line")
    if [[ -z $line ]]; then
        fail "$app: $menu has no $item ($(cut -f1 "$logs/$app-$menu.txt" | paste -sd, -))"
    elif [[ $got_char == "$char" && $got_mods == "$mods" ]]; then
        pass "$app: $menu → $item shows its key ($char, modifiers $mods)"
    else
        fail "$app: $menu → $item shows ${got_char:-no key} (modifiers ${got_mods:-?}), not $char ($mods)"
    fi
}

# Click menu $1 of the menu bar open (a name, or `2` for the app menu), screenshot, close.
show_menu() {
    local app=$1 menu=$2 name=$3 item
    if [[ $menu =~ ^[0-9]+$ ]]; then item="menu bar item $menu"; else item="menu bar item \"$menu\""; fi
    proc "click $item of menu bar 1"
    shot "$app-$name" 1
    code 53
}

# UPD-3: the app menu's Update Automatically: its check mark (✓, else nothing).
update_mark() {
    proc "get value of attribute \"AXMenuItemMarkChar\" of menu item \"Update Automatically\" of menu 1 of menu bar item 2 of menu bar 1" "$logs/$1-update-mark.txt"
    tr -d '[:space:]' <"$logs/$1-update-mark.txt" | sed 's/^missingvalue$//'
}
# Click it, then whether the settings file says $2 (`true` or `false`) and the mark is $3.
update_click() {
    local app=$1 want=$2 mark=$3 got
    proc "click menu item \"Update Automatically\" of menu 1 of menu bar item 2 of menu bar 1"
    sleep 1
    got=$(cat "$settings" 2>/dev/null)
    if [[ $got == "automatic=$want" ]]; then pass "$app: a click on Update Automatically keeps automatic=$want in $settings"
    else fail "$app: after a click $settings holds “${got}”, not automatic=$want"; fi
    got=$(update_mark "$app")
    if [[ $got == "$mark" ]]; then pass "$app: Update Automatically now shows mark “${mark}”"
    else fail "$app: Update Automatically shows mark “${got}”, not “${mark}”"; fi
}

# Check app $1, started on file $2; $3: its menu bar's menus after the app menu; $4: the
# other menu to show; then lines "menu|item|key|modifiers" on stdin.
check() {
    local app=$1 file=$2 menus=$3 other=$4 menu item char mods before after
    local expected=()
    while IFS='|' read -r menu item char mods; do expected+=("$menu|$item|$char|$mods"); done
    # UPD-3: the settings and the record of an applied update, as an update leaves them.
    local name version cache
    name=$(tr '[:lower:]' '[:upper:]' <<<"${app:0:1}")${app:1}
    version=$("$bin/$app" --version | cut -d' ' -f2)
    settings="$HOME/Library/Application Support/dev.cronkite.$name/update"
    cache="$HOME/Library/Caches/dev.cronkite.$name/updates"
    rm -f "$settings"
    mkdir -p "$cache" && printf 'version=%s\n' "$version" >"$cache/applied"
    start "$app" "$file" || return
    shot "$app-1-window"
    if grep -qx "update: Updated to $name $version" "$logs/$app.log"; then
        pass "$app: the first start after an update says “Updated to $name ${version}” (shots/$app-1-window.png)"
    else fail "$app: no “update: Updated to $name ${version}” in logs/$app.log"; fi
    if [[ -e $cache/applied ]]; then fail "$app: the applied record is still there"
    else pass "$app: the applied record is gone"; fi

    # The system's title bar: its close, minimize, and full-screen buttons.
    proc "get subrole of every button of window 1" "$logs/$app-buttons.txt"
    proc "get name of window 1" "$logs/$app-title.txt"
    if grep -q AXCloseButton "$logs/$app-buttons.txt"; then
        pass "$app: the window has macOS's title bar ($(cat "$logs/$app-buttons.txt")), titled “$(cat "$logs/$app-title.txt")”"
    else
        fail "$app: no macOS title bar buttons ($(cat "$logs/$app-buttons.txt"))"
    fi

    # The menu bar: Apple, the app menu, the app's menus, Window.
    proc "get name of every menu bar item of menu bar 1" "$logs/$app-menu-bar.txt"
    local want="Apple, $app, $menus, Window"
    if [[ $(cat "$logs/$app-menu-bar.txt") == "$want" ]]; then
        pass "$app: the menu bar is $want"
    else
        fail "$app: the menu bar is $(cat "$logs/$app-menu-bar.txt"), not $want"
    fi

    # The app menu: About first, Quit last with ⌘Q.
    menu_items "$app" "$app"
    if [[ $(head -1 "$logs/$app-$app.txt" | cut -f1) == "About $app" ]]; then
        pass "$app: the app menu starts with About $app"
    else
        fail "$app: the app menu doesn't start with About ($(cut -f1 "$logs/$app-$app.txt" | paste -sd, -))"
    fi
    expect_key "$app" "$app" "Quit $app" Q 0
    # Update Automatically: right after About, on by default; a click turns it off.
    if [[ $(sed -n 2p "$logs/$app-$app.txt" | cut -f1) == "Update Automatically" ]]; then
        pass "$app: Update Automatically follows About $app"
    else
        fail "$app: the app menu's second item isn't Update Automatically ($(cut -f1 "$logs/$app-$app.txt" | paste -sd, -))"
    fi
    if [[ $(update_mark "$app") == ✓ ]]; then pass "$app: Update Automatically is checked by default"
    else fail "$app: Update Automatically isn't checked at first (“$(cat "$logs/$app-update-mark.txt")”)"; fi
    update_click "$app" false ""
    menu_items "$app" Window
    for item in Minimize Zoom; do
        if cut -f1 "$logs/$app-Window.txt" | grep -qx "$item"; then pass "$app: Window → $item"
        else fail "$app: Window has no $item ($(cut -f1 "$logs/$app-Window.txt" | paste -sd, -))"; fi
    done
    expect_key "$app" Window Minimize M 0

    # The app's menus, their keys.
    local listed=" "
    for line in "${expected[@]}"; do
        IFS='|' read -r menu item char mods <<<"$line"
        if [[ $listed != *" $menu "* ]]; then menu_items "$app" "$menu"; listed+="$menu "; fi
        expect_key "$app" "$menu" "$item" "$char" "$mods"
    done

    # Open: the app menu, File, and another, for the screenshots.
    show_menu "$app" 2 2-app-menu
    show_menu "$app" File 3-file-menu
    show_menu "$app" "$other" "4-$(tr '[:upper:]' '[:lower:]' <<<"$other")-menu"

    # About: a window more.
    before=$(windows "$pid" | wc -l)
    proc "click menu item 1 of menu 1 of menu bar item 2 of menu bar 1"
    shot "$app-5-about" 2
    windows "$pid" >"$logs/$app-windows-about.txt"
    after=$(wc -l <"$logs/$app-windows-about.txt")
    if ((after > before)); then pass "$app: About $app opened its dialog ($before -> $after windows)"
    else fail "$app: About $app opened nothing ($before -> $after windows)"; fi
    code 53
    sleep 1

    # ⌘Q quits.
    front "$pid"
    key q "command down"
    local i
    for i in $(seq 20); do kill -0 "$pid" 2>/dev/null || break; sleep 0.5; done
    if kill -0 "$pid" 2>/dev/null; then
        shot "$app-6-still-running"
        # What it's doing: its windows and its threads' stacks.
        windows "$pid" >"$logs/$app-windows-quit.txt"
        sample "$pid" 3 -file "$logs/$app-sample.txt" >/dev/null 2>&1
        fail "$app: ⌘Q didn't quit ($(wc -l <"$logs/$app-windows-quit.txt") windows; stacks in logs/$app-sample.txt)"
        stop
    else
        wait "$pid" 2>/dev/null
        pass "$app: ⌘Q quit"
    fi

    # The next start: Update Automatically still off, no notice; a click turns it on.
    start "$app" "$file" || return
    shot "$app-7-restarted"
    if [[ -z $(update_mark "$app") ]]; then pass "$app: Update Automatically stays off after a restart"
    else fail "$app: Update Automatically is checked again after a restart"; fi
    if grep -q "^update: Updated to" "$logs/$app.log"; then fail "$app: the notice showed again"
    else pass "$app: the next start shows no notice"; fi
    update_click "$app" true ✓
    stop
}

brick() {
    printf 'id,name,city\n1,Ada,Paris\n2,Linus,Austin\n' >"$files/people.csv"
    check brick "$files/people.csv" "File, Edit, Sheet, Format, Data" Format <<'EOF'
File|New|N|0
File|Open…|O|0
File|Close Window|W|0
File|Save|S|0
File|Save As…|S|1
Edit|Undo|Z|0
Edit|Redo|Z|1
Edit|Copy|C|0
Edit|Replace…|F|2
Format|Bold|B|0
Format|Strikethrough|5|0
EOF
}

slate() {
    printf 'Original line.\n' >"$files/notes.txt"
    check slate "$files/notes.txt" "File, Edit, Format, View" Format <<'EOF'
File|New|N|0
File|Open…|O|0
File|Save|S|0
File|Save As…|S|1
File|Close Window|W|0
Edit|Undo|Z|0
Edit|Redo|Z|1
Edit|Replace…|F|2
Format|Bold|B|0
Format|Bulleted List|8|1
EOF
}

frame() {
    cp "$root/apps/frame/crates/pptx/tests/data/fixture.pptx" "$files/deck.pptx"
    check frame "$files/deck.pptx" "File, Edit, Insert, Slide, View" Insert <<'EOF'
File|New|N|0
File|Open…|O|0
File|Close Window|W|0
File|Save|S|0
File|Save As…|S|1
Edit|Undo|Z|0
Edit|Redo|Z|1
Slide|New Slide|N|1
Slide|Duplicate Slide|D|0
View|Outline|O|1
EOF
}

etch() {
    wide_png "$files/wide.png"
    check etch "$files/wide.png" "File, Edit, View, Image" Image <<'EOF'
File|New Image…|N|0
File|Open…|O|0
File|Save|S|0
File|Close|W|0
Edit|Undo|Z|0
Edit|Redo|Z|1
EOF
}

for app in brick slate frame etch; do
    if [[ -x $bin/$app ]]; then "$app"; else fail "no $bin/$app"; fi
done
if ((failed)); then note "check_mac_ui: FAILED"; exit 1; fi
note "check_mac_ui: OK"
