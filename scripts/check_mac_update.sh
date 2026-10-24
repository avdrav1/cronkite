#!/usr/bin/env bash
# UPD-5 acceptance on a macOS runner: an installed app updates itself (ADR 0012).
#
#   scripts/check_mac_update.sh build OUT APP...   # arm64 Mac with Homebrew's GTK (as mac_bundle.sh)
#   scripts/check_mac_update.sh check OUT APP...   # the same Mac, Homebrew moved away
#
# `build` makes five releases of each APP, versions 99.0.1 to 99.0.5 (above any real
# release, so a real latest.json never counts as newer): the app's Cargo.toml version set
# for each build in this work tree only and put back after (nothing shipped is built so),
# `cargo build --release -p APP --features suite-update/test-key`, then
# scripts/mac_bundle.sh. It writes OUT/site/APP/vN/ with the tarball and a latest.json signed
# by the update checks' test key (CRONKITE_RELEASE_TEST_KEY=1, never a release key), and
# three bad releases of 99.0.5's tarball offered as 99.0.6: bad-file (a byte of the tarball
# changed after signing), bad-sig (the manifest changed after signing), and changed (good,
# for a staged file changed on disk). The site's port is in OUT/port.
#
# `check` serves OUT/site on 127.0.0.1, installs 99.0.1 with the app's real install.sh, then
# starts the installed app with CRONKITE_UPDATE_URL pointing at a newer release and proves:
#   1. a staged update left by a killed process is applied at the next start, which swaps
#      the bundle before any window and runs the new executable in its own place (same
#      pid): 99.0.2 staged, the app killed (-9), then started by Launch Services (`open -a`
#      with a file, as Finder does: the file arrives by Apple Event after the re-exec);
#      again with 99.0.3 started from the command line (the file kept in the arguments);
#   2. a second running instance defers the swap: two instances run while 99.0.4 is staged;
#      quitting the first (⌘Q) leaves 99.0.3 installed, quitting the second swaps 99.0.4 in;
#   3. quitting then starting runs the new version: 99.0.5 staged, ⌘Q swaps it in, and the
#      next start is 99.0.5 with the notice: its banner (screenshot) and the line
#      "update: Updated to <App> 99.0.5" on stderr, once (each start after a swap has it);
#   after each swap the bundle's ad-hoc signature verifies (`codesign --verify --deep
#   --strict`), it carries no quarantine attribute, and nothing is left beside it;
#   4. tampered releases are refused, 99.0.5 stays and still starts: a changed tarball, a
#      changed manifest, and a staged file changed on disk after its check (the apply step's
#      own SHA-256 check).
# Screenshots land in $OUT_CHECK/shots (default out/mac-update), logs in $OUT_CHECK/logs; a
# failure exits 1 after every app has run.
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
mode=${1:-}
work=${2:?usage: check_mac_update.sh build|check OUT APP...}
shift 2
apps=${*:-brick slate frame etch}
mkdir -p "$work"
work=$(cd "$work" && pwd)
versions="1 2 3 4 5"
last=5
version() { echo "99.0.$1"; }
name_of() { echo "$(tr '[:lower:]' '[:upper:]' <<<"${1:0:1}")${1:1}"; }

build() {
  set -e
  local port=${PORT:-18731} app n v toml site tar dmg key=$root/crates/suite-update/test-key/test.key
  echo "$port" >"$work/port"
  # release_key.py signs through OpenSSL 3 (Ed25519); macOS's own openssl is LibreSSL.
  if ! openssl version | grep -q '^OpenSSL 3'; then PATH=$(brew --prefix openssl@3)/bin:$PATH; fi
  openssl version
  cp "$root/Cargo.lock" "$work/Cargo.lock.orig"
  for app in $apps; do
    toml=$root/apps/$app/Cargo.toml
    cp "$toml" "$work/$app-Cargo.toml.orig"
    trap 'cp "$work/$app-Cargo.toml.orig" "$toml"; cp "$work/Cargo.lock.orig" "$root/Cargo.lock"' EXIT
    for n in $versions; do
      v=$(version "$n")
      # The package's own version line (Brick's inherits the workspace's).
      awk -v v="$v" '!done && /^version[ .]/ { print "version = \"" v "\""; done = 1; next } { print }' \
        "$work/$app-Cargo.toml.orig" >"$toml"
      cargo build --release -p "$app" --features suite-update/test-key
      [[ $("$root/target/release/$app" --version) == "$app $v" ]] || { echo "built $("$root/target/release/$app" --version), not $v"; exit 1; }
      scripts_out=$work/build/v$n
      "$root/scripts/mac_bundle.sh" "$app" "$root/target/release" "$scripts_out" | tail -3
      tar=$scripts_out/$app-$v-arm64-macos.tar.gz
      dmg=$scripts_out/$app-$v-arm64-macos.dmg
      site=$work/site/$app/v$n
      mkdir -p "$site"
      cp "$tar" "$site/"
      # latest.json's top-level fields are Linux's; this check has no Linux build.
      echo "not built" >"$site/$app-$v-x86_64-linux.tar.gz"
      CRONKITE_RELEASE_TEST_KEY=1 CRONKITE_RELEASE_KEY=$key python3 "$root/scripts/release_json.py" write \
        "$app" "http://127.0.0.1:$port/$app/v$n" "$v" "$site/$app-$v-x86_64-linux.tar.gz" "$tar" "$dmg" none "$site/latest.json"
    done
    cp "$work/$app-Cargo.toml.orig" "$toml"
    # The bad releases, from the last one's tarball offered as the next version.
    v=$(version $((last + 1)))
    tar=$work/build/v$last/$app-$(version $last)-arm64-macos.tar.gz
    for kind in bad-file bad-sig changed; do
      site=$work/site/$app/$kind
      mkdir -p "$site"
      cp "$tar" "$site/$app-$v-arm64-macos.tar.gz"
      echo "not built" >"$site/$app-$v-x86_64-linux.tar.gz"
      CRONKITE_RELEASE_TEST_KEY=1 CRONKITE_RELEASE_KEY=$key python3 "$root/scripts/release_json.py" write \
        "$app" "http://127.0.0.1:$port/$app/$kind" "$v" "$site/$app-$v-x86_64-linux.tar.gz" \
        "$site/$app-$v-arm64-macos.tar.gz" "${tar%.tar.gz}.dmg" none "$site/latest.json"
    done
    # Same size, one byte changed in the middle: only the SHA-256 tells.
    python3 - "$work/site/$app/bad-file/$app-$v-arm64-macos.tar.gz" <<'EOF'
import sys
p = sys.argv[1]
b = bytearray(open(p, "rb").read())
b[len(b) // 2] ^= 0xFF
open(p, "wb").write(b)
EOF
    sed -i '' "s/\"version\":\"$v\"/\"version\":\"99.0.9\"/" "$work/site/$app/bad-sig/latest.json"
    grep -q '"version":"99.0.9"' "$work/site/$app/bad-sig/latest.json"
    cp "$root/sites/$app/install.sh" "$work/site/$app-install.sh"
  done
  cp "$work/Cargo.lock.orig" "$root/Cargo.lock"
  trap - EXIT
  echo "check_mac_update build: $apps in $work/site (port $port)"
}

check() {
  local port
  port=$(cat "$work/port")
  out=${OUT_CHECK:-out/mac-update}
  mkdir -p "$out/shots" "$out/logs" "$out/files"
  out=$(cd "$out" && pwd)
  shots=$out/shots logs=$out/logs files=$out/files bin=$out
  # shellcheck source=scripts/mac_helpers.sh
  source "$root/scripts/mac_helpers.sh"
  unset GSK_RENDERER # the apps choose cairo themselves on macOS (suite_ui::renderer)
  if [[ -e /opt/homebrew/lib ]]; then
    fail "Homebrew is at /opt/homebrew: this check needs a Mac without it"
  else
    pass "no Homebrew at /opt/homebrew ($(sw_vers -productVersion), $(uname -m))"
  fi
  printf 'id,name,city\n1,Ada,Paris\n2,Linus,Austin\n' >"$files/people.csv"
  printf '# Read me\n\nUpdated by itself.\n' >"$files/readme.md"
  cp "$root/apps/frame/crates/pptx/tests/data/fixture.pptx" "$files/deck.pptx"
  wide_png "$files/wide.png"

  python3 -m http.server "$port" --bind 127.0.0.1 --directory "$work/site" >"$logs/http.log" 2>&1 &
  server=$!
  trap 'kill $server 2>/dev/null' EXIT
  sleep 1

  for app in $apps; do
    case $app in
      brick) check_app brick "$files/people.csv" ;;
      slate) check_app slate "$files/readme.md" ;;
      frame) check_app frame "$files/deck.pptx" ;;
      etch) check_app etch "$files/wide.png" ;;
    esac
  done
  if ((failed)); then note "check_mac_update: FAILED"; exit 1; fi
  note "check_mac_update: OK"
}

# Check app $1, started on file $2 each time.
check_app() {
  local app=$1 file=$2 name appdir exe id updates url pid a b n log up f=""
  name=$(name_of "$app")
  appdir=$HOME/Applications/$name.app exe=$HOME/Applications/$name.app/Contents/MacOS/$app
  id=dev.cronkite.$name updates=$HOME/Library/Caches/dev.cronkite.$name/updates
  url=http://127.0.0.1:$port/$app
  up=$(tr '[:lower:]' '[:upper:]' <<<"$app")

  # Start the installed app on $file against release folder $1, logging to $2; sets $pid.
  # With $3 `open`, through Launch Services as Finder does (`open -a`): the file arrives
  # by Apple Event, not on the command line.
  launch() {
    if [[ ${3:-} == open ]]; then
      : >"$2"
      open -a "$appdir" --stderr "$2" --env "CRONKITE_UPDATE_URL=$url/$1" "$file" 2>>"$2" ||
        { fail "$app: open -a failed: $(tail -3 "$2" | tr '\n' '|')"; return 1; }
      pid=""
      local j
      for j in $(seq 20); do
        pid=$(pgrep -f "^$appdir/Contents/MacOS/" | head -1)
        [[ -n $pid ]] && break
        sleep 0.5
      done
      [[ -n $pid ]] || { fail "$app: open -a started nothing"; return 1; }
    else
      CRONKITE_UPDATE_URL=$url/$1 "$exe" "$file" >"$2" 2>&1 &
      pid=$!
    fi
    local i
    for i in $(seq 40); do
      [[ -n $(windows "$pid" | awk -F'\t' '$1 == 0') ]] && break
      kill -0 "$pid" 2>/dev/null || break
      sleep 0.5
    done
    sleep 1
    if [[ -n $(windows "$pid" | awk -F'\t' '$1 == 0') ]]; then return 0; fi
    fail "$app: no window (log $(basename "$2"): $(tail -5 "$2" | tr '\n' '|'))"
    return 1
  }
  # ⌘Q to process $1, once more if it's still there; it must have exited.
  quit() {
    local p=$1 try i
    for try in 1 2; do
      front "$p"
      key q "command down"
      for i in $(seq 20); do kill -0 "$p" 2>/dev/null || break; sleep 0.5; done
      kill -0 "$p" 2>/dev/null || { wait "$p" 2>/dev/null; return 0; }
    done
    fail "$app: ⌘Q didn't quit pid $p"
    kill -9 "$p" 2>/dev/null
    wait "$p" 2>/dev/null
    return 1
  }
  # Wait up to 60 s for version $1 to be staged.
  staged() {
    local i
    for i in $(seq 120); do
      if grep -qx "version=$1" "$updates/staged" 2>/dev/null; then
        local f
        f=$(sed -n 's/^file=//p' "$updates/staged")
        [[ -f $updates/$f ]] && { sleep 2; return 0; }
      fi
      sleep 0.5
    done
    return 1
  }
  installed() { "$exe" --version 2>/dev/null | cut -d' ' -f2; }
  # The installed bundle: version $1, signature valid, no quarantine, nothing beside it.
  bundle_ok() {
    local v=$1 what=$2 got
    got=$(installed)
    [[ $got == "$v" ]] && pass "$app: $what: the installed bundle is $v" ||
      fail "$app: $what: the installed bundle is ${got:-broken}, not $v"
    if codesign --verify --deep --strict "$appdir" 2>"$logs/$app-codesign.log"; then
      pass "$app: $what: codesign --verify --deep --strict passes ($(codesign -dv "$appdir" 2>&1 | grep -E '^Signature' | tr '\n' ' '))"
    else fail "$app: $what: codesign --verify: $(tr '\n' '|' <"$logs/$app-codesign.log")"; fi
    if xattr -r "$appdir" 2>/dev/null | grep -q com.apple.quarantine; then
      fail "$app: $what: quarantined ($(xattr -p com.apple.quarantine "$appdir" 2>&1))"
    else pass "$app: $what: no com.apple.quarantine (xattr -p: $(xattr -p com.apple.quarantine "$appdir" 2>&1 | tr '\n' ' '))"; fi
    local beside
    beside=$(ls -A "$HOME/Applications" | grep -F ".$name.app." || true)
    [[ -z $beside ]] && pass "$app: $what: nothing left beside $name.app" ||
      fail "$app: $what: left beside it: $beside"
  }
  # The notice for version $1 in the start logged at $2: the app logs
  # "update: Updated to <App> X" when its first window shows the banner (UPD-3), which
  # consumes the `applied` record.
  notice_ok() {
    local v=$1 log=$2
    if grep -q "update: Updated to $name $v" "$log" && [[ ! -e $updates/applied ]]; then
      pass "$app: the start says \"$(grep -o "Updated to $name $v" "$log" | head -1)\" (record consumed)"
    else
      fail "$app: no notice for $v: $(grep 'update:' "$log" | tr '\n' '|'); applied: $(cat "$updates/applied" 2>&1)"
    fi
  }

  rm -rf "$appdir" "$HOME/Library/Caches/$id"
  # 0. The old build, installed by the real installer.
  if curl -fsSL "http://127.0.0.1:$port/$app-install.sh" |
    env "${up}_DOWNLOAD_URL=$url/v1" sh >"$logs/$app-install.log" 2>&1 &&
    grep -q "^Installed $name $(version 1)" "$logs/$app-install.log"; then
    pass "$app: install.sh installed $(installed) in $appdir"
  else fail "$app: install: $(tr '\n' '|' <"$logs/$app-install.log")"; return; fi
  bundle_ok "$(version 1)" "installed"

  # 1. A staged update left by a killed process: applied at the next start, then re-exec;
  # version $1 (the installed one is $1 - 1), started the next time by $2 (open or exe).
  killed() {
    local n=$1 how=$2 v old tag
    v=$(version "$n") old=$(version $((n - 1))) tag=$app-1-$how
    log=$logs/$tag-killed.log
    launch "v$n" "$log" || return 1
    if staged "$v"; then pass "$app: $v staged while $old runs"
    else fail "$app: $v wasn't staged: $(grep 'update' "$log" | tr '\n' '|')"; kill -9 "$pid"; return 1; fi
    kill -9 "$pid"
    wait "$pid" 2>/dev/null
    [[ $(installed) == "$old" ]] && pass "$app: killed: still $old installed, $v staged" ||
      fail "$app: killed, yet $(installed) installed"
    log=$logs/$tag-next-start.log
    if [[ $how == open ]]; then launch "v$n" "$log" open || return 1; else launch "v$n" "$log" || return 1; fi
    shot "$tag-started-after-kill"
    if grep -q "update: .* $v swapped in" "$log" &&
      grep -q "update: .* $v started in place of $old" "$log" &&
      (($(grep -c "$name starting" "$log") == 2)); then
      pass "$app: the next start ($how) swapped $v in, then ran it in its own place (pid $pid): $(grep 'update:' "$log" | sed 's/.*update: /update: /' | tr '\n' '|')"
    else fail "$app: next start ($how): $(grep -E 'update:|starting' "$log" | tr '\n' '|')"; fi
    windows "$pid" | awk -F'\t' '$1 == 0' >"$logs/$tag-windows.txt"
    local from="its arguments"
    [[ $how == open ]] && from="Launch Services' Apple Event"
    grep -qF "$(basename "$file")" "$logs/$tag-windows.txt" &&
      pass "$app: the relaunched $v opened $(basename "$file") from $from: $(tr '\n' '|' <"$logs/$tag-windows.txt")" ||
      fail "$app: the relaunched $v's windows ($how): $(tr '\n' '|' <"$logs/$tag-windows.txt")"
    notice_ok "$v" "$log"
    bundle_ok "$v" "after the start's swap ($how)"
    quit "$pid"
  }
  killed 2 open || return
  killed 3 exe || return

  # 2. A second instance defers the swap until both have quit.
  local v4 v3
  v4=$(version 4) v3=$(version 3)
  log=$logs/$app-2-first.log
  launch v4 "$log" || return
  a=$pid
  launch v4 "$logs/$app-2-second.log" || { quit "$a"; return; }
  b=$pid
  if staged "$v4"; then pass "$app: $v4 staged while two instances run (pids $a, $b)"
  else fail "$app: $v4 wasn't staged"; fi
  shot "$app-2-two-instances"
  quit "$a"
  sleep 1
  [[ $(installed) == "$v3" ]] && grep -qx "version=$v4" "$updates/staged" &&
    pass "$app: the first quit with the second running: still $v3, $v4 waits" ||
    fail "$app: after the first quit: $(installed) installed; $(grep 'update:' "$log" | tr '\n' '|')"
  quit "$b"
  grep -q "update: .* $v4 swapped in" "$logs/$app-2-second.log" &&
    pass "$app: the last instance's quit swapped $v4 in: $(grep 'swapped in' "$logs/$app-2-second.log" | sed 's/.*update: /update: /')" ||
    fail "$app: the last quit: $(grep 'update:' "$logs/$app-2-second.log" | tr '\n' '|')"
  bundle_ok "$v4" "after both quit"
  log=$logs/$app-2-next-start.log
  launch v4 "$log" && {
    shot "$app-2-updated"
    notice_ok "$v4" "$log"
    quit "$pid"
  }

  # 3. Quit, then start: the new version, with the notice.
  local v5
  v5=$(version $last)
  log=$logs/$app-3-quit.log
  launch "v$last" "$log" || return
  staged "$v5" && pass "$app: $v5 staged" || fail "$app: $v5 wasn't staged"
  quit "$pid"
  grep -q "update: .* $v5 swapped in" "$log" &&
    pass "$app: ⌘Q swapped $v5 in" || fail "$app: ⌘Q: $(grep 'update:' "$log" | tr '\n' '|')"
  bundle_ok "$v5" "after quitting"
  # Launch Services has the new version's record (Finder's Get Info): the swap registers
  # the bundle again (lsregister -f), else its record stays the old one until the app runs.
  local ls=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
  "$ls" -dump 2>/dev/null | awk -v p="$appdir" 'index($0, "path:") && index($0, p) && !done {on = 1} on && /^-+$/ {on = 0; done = 1} on' >"$logs/$app-3-lsregister.txt"
  if grep -Eq "^versionString: *$v5\$" "$logs/$app-3-lsregister.txt"; then
    pass "$app: Launch Services has $v5 after the swap: $(grep -E '^(path|versionString):' "$logs/$app-3-lsregister.txt" | tr -s ' ' | tr '\n' '|')"
  else fail "$app: Launch Services after the swap: $(grep -E '^(path|versionString):' "$logs/$app-3-lsregister.txt" | tr -s ' ' | tr '\n' '|')"; fi
  log=$logs/$app-3-next-start.log
  launch "v$last" "$log" || return
  shot "$app-3-updated"
  notice_ok "$v5" "$log"
  ! grep -q "swapped in\|started in place" "$log" && pass "$app: the next start is $v5 without swapping anything" ||
    fail "$app: the next start swapped again: $(grep 'update:' "$log" | tr '\n' '|')"
  quit "$pid"

  # 4. Tampered releases: refused, the installed app stays and starts.
  local bad
  bad=$(version $((last + 1)))
  for kind in bad-file bad-sig; do
    log=$logs/$app-4-$kind.log
    launch "$kind" "$log" || return
    for n in $(seq 40); do grep -q "WARN.*update:" "$log" && break; sleep 0.5; done
    quit "$pid"
    if grep -qE "WARN.*update: .*(sha256|signature)" "$log" && ! grep -qx "version=$bad" "$updates/staged" 2>/dev/null; then
      pass "$app: $kind refused: $(grep 'WARN.*update:' "$log" | sed 's/.*update: /update: /' | head -1)"
    else fail "$app: $kind: $(grep 'update:' "$log" | tr '\n' '|'); staged: $(cat "$updates/staged" 2>&1 | tr '\n' ' ')"; fi
    bundle_ok "$v5" "after $kind"
  done
  log=$logs/$app-4-staged-changed.log
  launch changed "$log" || return
  if staged "$bad"; then
    f=$updates/$(sed -n 's/^file=//p' "$updates/staged")
    python3 -c 'import sys; p = sys.argv[1]; b = bytearray(open(p, "rb").read()); b[len(b) // 2] ^= 0xFF; open(p, "wb").write(b)' "$f"
    pass "$app: $bad staged, then a byte of $(basename "$f") changed"
  else fail "$app: $bad wasn't staged"; fi
  quit "$pid"
  grep -q "WARN.*update: not applied: the staged .* has sha256" "$log" && [[ ! -f $f ]] &&
    pass "$app: the changed staged file refused and removed at exit: $(grep 'not applied' "$log" | sed 's/.*update: /update: /')" ||
    fail "$app: changed staged file: $(grep 'update:' "$log" | tr '\n' '|')"
  bundle_ok "$v5" "after the changed staged file"
  log=$logs/$app-4-still-starts.log
  launch "v$last" "$log" && {
    shot "$app-4-still-starts"
    pass "$app: $(installed) still starts after the tampered releases"
    ! grep -q "Updated to" "$logs"/$app-4-*.log && pass "$app: no notice at the starts after the one that showed it" ||
      fail "$app: a notice again: $(grep -h 'Updated to' "$logs"/$app-4-*.log | tr '\n' '|')"
    quit "$pid"
  }

  if curl -fsSL "http://127.0.0.1:$port/$app-install.sh" | env "${up}_DOWNLOAD_URL=$url/v1" sh -s -- --uninstall >"$logs/$app-uninstall.log" 2>&1; then
    note "$app: uninstalled"
  fi
}

case $mode in
  build) build ;;
  check) check ;;
  *) echo "usage: check_mac_update.sh build|check OUT APP..."; exit 2 ;;
esac
