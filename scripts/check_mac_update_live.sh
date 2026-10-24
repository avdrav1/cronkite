#!/usr/bin/env bash
# UPD-7 on macOS: an installed copy of a live release updates itself from the live
# latest.json, in the wild (ADR 0012). On an arm64 Mac without Homebrew (update-live.yml's
# macos job moves it away), for each APP:
#
#   python3 scripts/pin_release.py SITE brick:0.6.0 ...   # the site the installers read
#   scripts/check_mac_update_live.sh SITE [APP...]
#
#   1. the app's real installer (https://<app>.cronkite.cc/install.sh, as SITE/<app> holds
#      it) installs the live release FROM named by SITE/<app>/latest.json, served on
#      127.0.0.1 (APP_DOWNLOAD_URL; the installer checks the tarball's sha256): ~/Applications/
#      <App>.app; `--version` is recorded;
#   2. Launch Services starts it (`open -a`, on a file, stderr kept) with no
#      CRONKITE_UPDATE_URL and no test key: it checks https://dl.cronkite.cc's latest.json,
#      signed with a release key;
#   with EXPECT_UPDATE=1 (the default), the live latest must be newer than FROM, and:
#   3. while FROM runs it logs "update: <App> LATEST downloaded for the next start" and
#      stages LATEST (screenshot); ⌘Q swaps it in at the exit ("... swapped in");
#   4. `--version` is LATEST (recorded), the bundle's signature verifies, no quarantine;
#   5. the next start logs "update: Updated to <App> LATEST" with the banner (screenshot)
#      and consumes the record;
#   with EXPECT_UPDATE=0 the live latest must be FROM itself (before the next release), and:
#   3. the check runs to its end (updates/.lock taken, then released) without a problem
#      logged and stages nothing: up to date (screenshot); after ⌘Q `--version` is still
#      FROM.
# It replaces ~/Applications/<App>.app: run it where no copy you keep is installed (CI).
# Screenshots land in $OUT_CHECK/shots (default out/update-live-mac), logs in
# $OUT_CHECK/logs, the results and recorded versions in $OUT_CHECK/summary.txt; ends with
# "check_mac_update_live: OK (N checks)" or exits 1 after every app has run.
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
site=$(cd "${1:?usage: check_mac_update_live.sh SITE [APP...]}" && pwd)
shift
apps=${*:-brick slate frame etch}
expect=${EXPECT_UPDATE:-1}
out=${OUT_CHECK:-out/update-live-mac}
mkdir -p "$out/shots" "$out/logs" "$out/files"
out=$(cd "$out" && pwd)
: >"$out/summary.txt"
shots=$out/shots logs=$out/logs files=$out/files bin=$out
# shellcheck source=scripts/mac_helpers.sh
source "$root/scripts/mac_helpers.sh"
unset GSK_RENDERER # the apps choose cairo themselves on macOS (suite_ui::renderer)
oks=0
pass() { oks=$((oks + 1)); note "ok   $*"; }
name_of() { echo "$(tr '[:lower:]' '[:upper:]' <<<"${1:0:1}")${1:1}"; }
live() { if [[ $1 == brick ]]; then echo https://dl.cronkite.cc; else echo "https://dl.cronkite.cc/$1"; fi; }
version_in() { python3 -c 'import json, sys; print(json.load(sys.stdin)["version"])'; }

note "UPD-7 in the wild, macOS $(sw_vers -productVersion) ($(uname -m)), $(date -u +%Y-%m-%dT%H:%MZ), EXPECT_UPDATE=$expect"
if [[ -e /opt/homebrew/lib ]]; then fail "Homebrew is at /opt/homebrew: this check needs a Mac without it"
else pass "no Homebrew at /opt/homebrew"; fi
printf 'id,name,city\n1,Ada,Paris\n2,Linus,Austin\n' >"$files/people.csv"
printf '# Read me\n\nUpdated by itself.\n' >"$files/readme.md"
cp "$root/apps/frame/crates/pptx/tests/data/fixture.pptx" "$files/deck.pptx"
wide_png "$files/wide.png"

port=$((20000 + RANDOM % 20000))
python3 -m http.server "$port" --bind 127.0.0.1 --directory "$site" >"$logs/http.log" 2>&1 &
server=$!
trap 'kill $server 2>/dev/null' EXIT
for i in $(seq 40); do curl -fsS -o /dev/null "http://127.0.0.1:$port/" && break; sleep 0.25; done

# Check app $1, started on file $2 each time.
check_app() {
  local app=$1 file=$2 name appdir exe id updates up from latest log pid
  name=$(name_of "$app")
  appdir=$HOME/Applications/$name.app exe=$HOME/Applications/$name.app/Contents/MacOS/$app
  id=dev.cronkite.$name updates=$HOME/Library/Caches/dev.cronkite.$name/updates
  up=$(tr '[:lower:]' '[:upper:]' <<<"$app")
  from=$(version_in <"$site/$app/latest.json") || { fail "$app: no pinned release in $site/$app/latest.json"; return; }
  latest=$(curl -fsS "$(live "$app")/latest.json" | version_in) || { fail "$app: no live latest.json at $(live "$app")"; return; }
  note "== $name: FROM $from, live latest $latest"
  if [[ $expect == 1 && $latest == "$from" ]]; then
    fail "$app: no newer release live: $(live "$app")/latest.json is $from (EXPECT_UPDATE=0 checks the up-to-date path)"
    return
  elif [[ $expect == 0 && $latest != "$from" ]]; then
    fail "$app: $(live "$app")/latest.json is $latest, not $from: EXPECT_UPDATE=0 expects FROM to be the live latest"
    return
  fi

  # Start the installed app on $file through Launch Services, its stderr in $1; sets $pid.
  launch() {
    log=$1
    : >"$log"
    open -a "$appdir" --stderr "$log" "$file" 2>>"$log" || { fail "$app: open -a failed: $(tail -3 "$log" | tr '\n' '|')"; return 1; }
    pid=""
    local i
    for i in $(seq 20); do
      pid=$(pgrep -f "^$appdir/Contents/MacOS/" | head -1)
      [[ -n $pid ]] && break
      sleep 0.5
    done
    [[ -n $pid ]] || { fail "$app: open -a started nothing"; return 1; }
    for i in $(seq 40); do
      [[ -n $(windows "$pid" | awk -F'\t' '$1 == 0') ]] && break
      kill -0 "$pid" 2>/dev/null || break
      sleep 0.5
    done
    sleep 1
    [[ -n $(windows "$pid" | awk -F'\t' '$1 == 0') ]] && return 0
    fail "$app: no window (log $(basename "$log"): $(tail -5 "$log" | tr '\n' '|'))"
    return 1
  }
  # ⌘Q, once more if it's still there; it must have exited.
  quit() {
    local try i
    for try in 1 2; do
      front "$pid"
      key q "command down"
      for i in $(seq 20); do kill -0 "$pid" 2>/dev/null || break; sleep 0.5; done
      kill -0 "$pid" 2>/dev/null || return 0
    done
    fail "$app: ⌘Q didn't quit pid $pid"
    kill -9 "$pid" 2>/dev/null
    return 1
  }
  saw() { # PATTERN SECONDS: in $log
    local i
    for i in $(seq $(($2 * 2))); do grep -q -- "$1" "$log" && return 0; sleep 0.5; done
    return 1
  }
  installed() { "$exe" --version 2>/dev/null | cut -d' ' -f2; }
  lines() { grep -o 'update: .*' "$log" | tr '\n' '|'; }
  # The check ran to its end: it takes updates/.lock (made then) for as long as it runs.
  finished() {
    local i
    for i in $(seq 120); do [[ -e $updates/.lock ]] && break; sleep 0.5; done
    [[ -e $updates/.lock ]] || return 1
    sleep 2
    for i in $(seq 240); do
      python3 -c 'import fcntl, sys; fcntl.flock(open(sys.argv[1], "a"), fcntl.LOCK_EX | fcntl.LOCK_NB)' "$updates/.lock" 2>/dev/null && return 0
      sleep 0.5
    done
    return 1
  }

  rm -rf "$appdir" "$HOME/Library/Caches/$id"
  if curl -fsSL "http://127.0.0.1:$port/$app/install.sh" |
    env "${up}_DOWNLOAD_URL=http://127.0.0.1:$port/$app" sh >"$logs/$app-install.log" 2>&1 &&
    grep -q "^Installed $name $from\$" "$logs/$app-install.log"; then
    pass "$app: https://$app.cronkite.cc/install.sh installed the live $name $from in $appdir"
  else fail "$app: install: $(tr '\n' '|' <"$logs/$app-install.log")"; return; fi
  note "$app: --version before: $("$exe" --version 2>&1)"
  [[ $(installed) == "$from" ]] && pass "$app: --version is $from" || fail "$app: --version is $(installed), not $from"

  launch "$logs/$app-1-$from.log" || return
  if [[ $expect == 1 ]]; then
    if saw "update: $name $latest downloaded for the next start" 300 && grep -qx "version=$latest" "$updates/staged"; then
      pass "$app: the running $from downloaded and staged the live $latest: $(lines)"
    else fail "$app: $latest wasn't staged: $(lines); staged: $(cat "$updates/staged" 2>&1 | tr '\n' ' ')"; fi
    shot "$app-1-$from-updating"
    quit
    saw "update: .* $latest swapped in" 10 && pass "$app: ⌘Q swapped $latest in: $(lines)" ||
      fail "$app: ⌘Q: $(lines)"
    note "$app: --version after: $("$exe" --version 2>&1)"
    [[ $(installed) == "$latest" ]] && pass "$app: after quitting, --version is $latest" ||
      fail "$app: after quitting, --version is $(installed), not $latest"
    if codesign --verify --deep --strict "$appdir" 2>"$logs/$app-codesign.log"; then
      pass "$app: the swapped bundle's signature verifies (codesign --verify --deep --strict)"
    else fail "$app: codesign --verify: $(tr '\n' '|' <"$logs/$app-codesign.log")"; fi
    if xattr -r "$appdir" 2>/dev/null | grep -q com.apple.quarantine; then fail "$app: the swapped bundle is quarantined"
    else pass "$app: the swapped bundle carries no quarantine"; fi

    launch "$logs/$app-2-$latest.log" || return
    if saw "update: Updated to $name $latest" 30 && [[ ! -e $updates/applied ]]; then
      pass "$app: the next start says \"Updated to $name $latest\" (record consumed)"
    else fail "$app: no notice for $latest: $(lines); applied: $(cat "$updates/applied" 2>&1)"; fi
    shot "$app-2-$latest-updated"
    quit
  else
    if finished; then pass "$app: the check of the live latest.json ran to its end (updates/.lock taken, released)"
    else fail "$app: the check didn't run or didn't end: updates/: $(ls -A "$updates" 2>&1 | tr '\n' ' ')"; fi
    grep -q 'update:' "$log" && fail "$app: logged: $(lines)" ||
      pass "$app: it logged no problem: latest.json and its signature verified, nothing newer"
    # Nothing in updates/ but the check's .lock and the running instances' owner-PID.lock.
    local left
    left=$(ls "$updates" 2>/dev/null | grep -v '^owner-[0-9]*\.lock$')
    [[ -z $left ]] && pass "$app: nothing staged: $from is the live latest (updates/: $(ls -A "$updates" | tr '\n' ' '))" ||
      fail "$app: staged: $(tr '\n' ' ' <<<"$left")"
    shot "$app-1-$from-up-to-date"
    quit
    note "$app: --version after: $("$exe" --version 2>&1)"
    [[ $(installed) == "$from" ]] && pass "$app: after quitting, --version is still $from" ||
      fail "$app: after quitting, --version is $(installed), not $from"
  fi
}

for app in $apps; do
  case $app in
    brick) check_app brick "$files/people.csv" ;;
    slate) check_app slate "$files/readme.md" ;;
    frame) check_app frame "$files/deck.pptx" ;;
    etch) check_app etch "$files/wide.png" ;;
    *) fail "unknown app $app" ;;
  esac
done
if ((failed)) || ((oks == 0)); then note "check_mac_update_live: FAILED"; exit 1; fi
note "check_mac_update_live: OK ($oks checks)"
