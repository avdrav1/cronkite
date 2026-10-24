#!/usr/bin/env bash
# UPD-4 acceptance: on Linux each app updates itself (ADR 0012). In clean Arch, Ubuntu
# 24.04, and Fedora containers, for Brick, Slate, Frame, and Etch, as a normal user:
#
#   scripts/check_update.sh
#
#   1. its install.sh installs an OLD build (this tree's version) from a local site, which
#      makes it the default for its file types beside another app's default (text/plain);
#   2. the app starts (GTK's broadway backend) with CRONKITE_UPDATE_URL at a NEW release
#      (patch version + 1) signed with suite-update's test key; while it runs, its check
#      downloads, verifies, and installs the new release ("update: <App> NEW installed for
#      the next start"): `--version` already says NEW, the running process's binary is the
#      old one, unlinked (/proc/PID/exe "(deleted)"), and it keeps running; then it quits;
#   3. the installed binary, desktop entry (Exec the full path), icon, and licences are the
#      new tarball's, byte for byte (the new release's carry a marker the old one's don't),
#      updates/applied names NEW, and `xdg-mime query default` for its types and
#      text/plain, and mimeapps.list's bytes, are as before;
#   4. the next start runs NEW, shows the notice (NOTICE=line: logs `update: Updated to
#      <App> NEW` and consumes the record, once UPD-3 shows it; NOTICE=record, until then:
#      updates/applied still names NEW), and installs nothing again;
#   5. reinstalled OLD, a release whose tarball's bytes changed under a correctly signed
#      manifest is refused ("the download's sha256 is …"): nothing staged, OLD still
#      installed and running, defaults as before;
#   6. a manifest signed by another key is refused ("signature doesn't verify"): the same.
#
# Builds, in packaging/build.Dockerfile's Ubuntu 24.04 image as `scripts/ship.sh build`
# does, the four apps twice with `--features suite-update/test-key` (so they trust the test
# key; release builds never have it): OLD from this tree, NEW from a copy of it in
# target/check-update/new-src with every app's version bumped (this tree is never
# changed). Each release is a tarball laid out as ship.sh's (binary stripped, desktop
# entry, icon, README, LICENSE*, THIRD-PARTY-RUST.txt from scripts/rust_licenses.py), and
# its latest.json is written by scripts/release_json.py with CRONKITE_RELEASE_TEST_KEY=1
# and the test key; the mis-signed one is signed with a throwaway key made here.
#
# Needs docker, python3, openssl, rsync. APPS="etch brick" checks some apps; ONLY=Fedora (or
# Arch, Ubuntu) one distro; VERBOSE=1 prints each container's transcript. Exits nonzero on
# any failure; prints `check_update: OK (N checks)` when all pass.
set -euo pipefail
cd "$(dirname "$0")/.."
repo=$PWD
APPS=${APPS:-brick slate frame etch}
NOTICE=${NOTICE:-record}
TARGET=x86_64-unknown-linux-gnu
IMAGE=brick-build:ubuntu-24.04
work=target/check-update
declare -A ID=([brick]=dev.cronkite.Brick [slate]=dev.cronkite.Slate [frame]=dev.cronkite.Frame [etch]=dev.cronkite.Etch)
declare -A MANIFEST=([brick]=Cargo.toml [slate]=apps/slate/Cargo.toml [frame]=apps/frame/Cargo.toml [etch]=apps/etch/Cargo.toml)
declare -A SECTION=([brick]=workspace.package [slate]=package [frame]=package [etch]=package)
version_in() { sed -n "/^\[${SECTION[$1]}\]/,/^\[/{s/^version = \"\(.*\)\"/\1/p;}" "$2/${MANIFEST[$1]}"; }
declare -A OLD NEW
for a in $APPS; do
  OLD[$a]=$(version_in "$a" .)
  IFS=. read -r ma mi pa <<<"${OLD[$a]}"
  NEW[$a]="$ma.$mi.$((pa + 1))"
done

# The NEW tree: this tree's files (tracked and new, not ignored), versions bumped; synced
# by checksum so an unchanged file keeps its time and cargo rebuilds only what changed.
mkdir -p "$work"
stage="$work/new-stage"
git ls-files -z -co --exclude-standard | rsync -a --from0 --files-from=- --ignore-missing-args ./ "$stage/"
for a in $APPS; do
  sed -i "/^\[${SECTION[$a]}\]/,/^\[/{s/^version = \".*\"/version = \"${NEW[$a]}\"/;}" "$stage/${MANIFEST[$a]}"
  [ "$(version_in "$a" "$stage")" = "${NEW[$a]}" ] || { echo "couldn't bump $a's version"; exit 1; }
done
rsync -a --checksum --delete --exclude=/Cargo.lock "$stage/" "$work/new-src/"
[ -f "$work/new-src/Cargo.lock" ] || cp Cargo.lock "$work/new-src/"

echo "building ${APPS// /, } OLD and NEW with suite-update/test-key"
packages=""; for a in $APPS; do packages="$packages -p $a"; done
docker build -q -t "$IMAGE" -f packaging/build.Dockerfile packaging >/dev/null
docker run --rm -u "$(id -u):$(id -g)" -v "$PWD:/src" -w /src \
  -e CARGO_HOME=/src/target/container/cargo -e CARGO_TARGET_DIR=/src/$work/build "$IMAGE" sh -c "
    set -e
    cargo build -q --release --locked --target $TARGET $packages --features suite-update/test-key
    mkdir -p $work/bin/old $work/bin/new
    for a in $APPS; do cp $work/build/$TARGET/release/\$a $work/bin/old/; done
    cd $work/new-src
    cargo build -q --release --target $TARGET $packages --features suite-update/test-key
    for a in $APPS; do cp /src/$work/build/$TARGET/release/\$a /src/$work/bin/new/; done"

port=$((20000 + RANDOM % 20000))
url=http://127.0.0.1:$port
site="$work/site"; rm -rf "$site" "$work/pack"; mkdir -p "$site" "$work/pack" "$work/licences"
openssl genpkey -algorithm ed25519 -out "$work/wrong.key" 2>/dev/null

# pack APP VERSION KIND: KIND's tarball in the site, as ship.sh build lays it out; the NEW
# release's desktop entry, icon, and licences carry a marker the old ones don't.
pack() {
  local a=$1 v=$2 kind=$3 name="$1-$2-x86_64-linux" from=old
  [ "$kind" = old ] || from=new
  local d="$work/pack/$kind/$name"
  mkdir -p "$d"
  cp "$work/bin/$from/$a" "$d/"; strip "$d/$a"
  cp "packaging/${ID[$a]}.desktop" "packaging/${ID[$a]}.svg" "$d/"
  [ -f "apps/$a/README.md" ] && cp "apps/$a/README.md" "$d/"
  cp LICENSE* "$d/"
  cp "$work/licences/$a.txt" "$d/THIRD-PARTY-RUST.txt"
  if [ "$from" = new ]; then
    printf 'X-Cronkite-Update-Check=%s\n' "$v" >> "$d/${ID[$a]}.desktop"
    printf '<!-- update check %s -->\n' "$v" >> "$d/${ID[$a]}.svg"
    for f in LICENSE LICENSE-APACHE THIRD-PARTY-RUST.txt; do printf '\nupdate check %s\n' "$v" >> "$d/$f"; done
  fi
  mkdir -p "$site/$a/$kind/releases/v$v"
  tar -C "$work/pack/$kind" -czf "$site/$a/$kind/releases/v$v/$name.tar.gz" "$name"
  CRONKITE_RELEASE_TEST_KEY=1 CRONKITE_RELEASE_KEY="$repo/crates/suite-update/test-key/test.key" \
    python3 scripts/release_json.py write "$a" "$url/$a/$kind/releases/v$v" "$v" \
    "$site/$a/$kind/releases/v$v/$name.tar.gz" none none none "$site/$a/$kind/latest.json"
}
for a in $APPS; do
  [ -s "$work/licences/$a.txt" ] || python3 scripts/rust_licenses.py "$a" $TARGET "$work/licences/$a.txt" >/dev/null
  pack "$a" "${OLD[$a]}" old
  pack "$a" "${NEW[$a]}" new
  # The tarball's bytes changed after its manifest was signed: same size, another sha256.
  pack "$a" "${NEW[$a]}" tampered
  python3 - "$site/$a/tampered/releases/v${NEW[$a]}/$a-${NEW[$a]}-x86_64-linux.tar.gz" <<'EOF'
import sys
p = sys.argv[1]
b = bytearray(open(p, "rb").read())
b[len(b) // 2] ^= 0xFF
open(p, "wb").write(b)
EOF
  # Signed by a key no build trusts.
  pack "$a" "${NEW[$a]}" missigned
  openssl pkeyutl -sign -rawin -inkey "$work/wrong.key" -in "$site/$a/missigned/latest.json" |
    base64 -w0 > "$site/$a/missigned/latest.json.sig"
  cp "sites/$a/install.sh" "$site/$a/install.sh"
done
{
  echo "APPS='$APPS'"; echo "NOTICE=$NOTICE"
  for a in $APPS; do echo "OLD_$a=${OLD[$a]}"; echo "NEW_$a=${NEW[$a]}"; done
} > "$site/versions"
cat > "$site/run.sh" <<'EOF'
#!/bin/sh
# check_update.sh's steps in one container, as the user tester; prints ok/FAIL lines.
# URL: the site; DISPLAY: broadway's, its own per container (they share the host's network,
# where display N listens on port 8080+N).
url=$1 display=$2
. /site/versions
export GDK_BACKEND=broadway BROADWAY_DISPLAY=:$display GDK_DEBUG=no-portals
cd "$HOME"
gtk4-broadwayd ":$display" >/dev/null 2>&1 &
sleep 1
mkdir -p ~/.config
printf '[Default Applications]\ntext/plain=gedit.desktop\n' > ~/.config/mimeapps.list
DATA=$HOME/.local/share
BIN=$HOME/.local/bin

check() { # DESCRIPTION COMMAND...: one ok or FAIL line; a failure shows the current log
  d=$1; shift
  if "$@" >/dev/null 2>&1; then echo "ok   $app: $d"
  else echo "FAIL $app: $d"; [ -f "${log:-}" ] && tail -n 15 "$log" | sed 's/^/     | /'; fi
}
title() { case $1 in brick) echo Brick ;; slate) echo Slate ;; frame) echo Frame ;; etch) echo Etch ;; esac; }
types() {
  case $1 in
    brick) echo "text/csv text/tab-separated-values" ;;
    slate) echo "application/vnd.openxmlformats-officedocument.wordprocessingml.document text/markdown" ;;
    frame) echo "application/vnd.openxmlformats-officedocument.presentationml.presentation" ;;
    etch) echo "image/png image/jpeg image/bmp" ;;
  esac
}
# What decides default apps: each type's default as xdg-mime tells it, and mimeapps.list.
defaults() {
  for t in $(types "$app") text/plain; do echo "$t=$(xdg-mime query default "$t" 2>/dev/null)"; done
  sha256sum ~/.config/mimeapps.list
}
ours() { for t in $(types "$app"); do xdg-mime query default "$t" | grep -qx "$ID.desktop" || return 1; done; }
install_old() {
  env "$(echo "$app" | tr a-z A-Z)_DOWNLOAD_URL=$url/$app/old" sh "/site/$app/install.sh" 2>&1
}
start() { # KIND LOG: the installed app, checking KIND's release
  log=$2
  CRONKITE_UPDATE_URL=$url/$app/$1 "$BIN/$app" > "$log" 2>&1 &
  pid=$!
}
saw() { # PATTERN: in the log within 90 s, while the app runs
  i=0
  while [ $i -lt 180 ]; do
    grep -q -- "$1" "$log" && return 0
    kill -0 "$pid" 2>/dev/null || return 1
    sleep 0.5; i=$((i + 1))
  done
  return 1
}
running() { sleep 3; kill -0 "$pid"; }
quit() { kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null; }
version_is() { [ "$("$BIN/$app" --version 2>/dev/null)" = "$app $1" ]; }
unlinked() { readlink "/proc/$pid/exe" | grep -q " (deleted)$"; }
same() { [ "$1" = "$2" ]; }
said() { printf '%s\n' "$out" | grep -q -- "$1"; }
applied_is() { grep -qx "version=$1" ~/.cache/$app/updates/applied; }
nothing_staged() { [ ! -e ~/.cache/$app/updates/staged ] && ! ls ~/.cache/$app/updates/*.tar.gz; }
# The installed files are the new tarball's, byte for byte (desktop entry: Exec rewritten
# as install.sh does).
replaced() {
  src=/tmp/new-$app/$app-$new-x86_64-linux
  cmp "$src/$app" "$BIN/$app" &&
    cmp "$src/$ID.svg" "$DATA/icons/hicolor/scalable/apps/$ID.svg" &&
    sed "s|^Exec=$app |Exec=$BIN/$app |; s|^Exec=$app\$|Exec=$BIN/$app|" "$src/$ID.desktop" |
      cmp - "$DATA/applications/$ID.desktop" &&
    grep -qx "X-Cronkite-Update-Check=$new" "$DATA/applications/$ID.desktop" &&
    grep -q "^Exec=$BIN/$app" "$DATA/applications/$ID.desktop" &&
    for f in LICENSE LICENSE-APACHE THIRD-PARTY-RUST.txt; do cmp "$src/$f" "$DATA/doc/$app/$f" || return 1; done
}

for app in $APPS; do
  eval "old=\$OLD_$app new=\$NEW_$app"
  App=$(title "$app"); ID=dev.cronkite.$App
  log=""
  mkdir -p "/tmp/new-$app"
  tar -xzf "/site/$app/new/releases/v$new/$app-$new-x86_64-linux.tar.gz" -C "/tmp/new-$app"

  out=$(install_old)
  check "install.sh installs $old" said "Installed $App $old"
  check "--version is $old" version_is "$old"
  check "install.sh made it the default for $(types "$app")" ours
  before=$(defaults)

  start new "/tmp/$app-update.log"
  check "the running $old installs $new ('installed for the next start')" saw "update: $App $new installed for the next start"
  check "the check reports $new downloaded" saw "update: $App $new downloaded for the next start"
  check "--version is $new while $old runs" version_is "$new"
  check "the running process keeps its old binary, unlinked" unlinked
  check "$old keeps running after the update" running
  quit
  check "after quitting, --version is $new" version_is "$new"
  check "binary, desktop entry, icon, and licences are $new's" replaced
  check "the binary is executable" test -x "$BIN/$app"
  check "updates/applied names $new" applied_is "$new"
  check "default apps and mimeapps.list unchanged" same "$before" "$(defaults)"

  start new "/tmp/$app-next.log"
  check "the next start runs $new" running
  if [ "$NOTICE" = line ]; then
    check "the next start shows 'Updated to $App $new'" saw "update: Updated to $App $new"
    check "the notice's record is consumed" test ! -e ~/.cache/$app/updates/applied
  else
    check "the next start finds updates/applied naming $new for its notice" applied_is "$new"
  fi
  check "the next start installs nothing again" sh -c "! grep -q 'installed for the next start' '$log'"
  quit
  check "default apps unchanged after the next start" same "$before" "$(defaults)"

  for bad in tampered missigned; do
    rm -rf ~/.cache/$app
    out=$(install_old)
    check "$bad: install.sh reinstalls $old" said "$App $old"
    check "$bad: --version is $old before the check" version_is "$old"
    start $bad "/tmp/$app-$bad.log"
    if [ $bad = tampered ]; then
      check "tampered: the download is refused" saw "update: the download's sha256 is"
    else
      check "missigned: the manifest is refused" saw "update: latest.json's signature doesn't verify"
    fi
    check "$bad: $old keeps running" running
    check "$bad: nothing installed" sh -c "! grep -q 'installed for the next start' '$log'"
    quit
    check "$bad: nothing staged" nothing_staged
    check "$bad: --version is still $old" version_is "$old"
    start $bad "/tmp/$app-$bad-again.log"
    check "$bad: $old starts again" running
    quit
    check "$bad: default apps unchanged" same "$before" "$(defaults)"
  done
done
echo "run.sh: finished"
EOF
chmod -R a+rX "$site"
python3 -m http.server "$port" --bind 127.0.0.1 --directory "$site" >/dev/null 2>&1 &
server=$!
trap 'kill $server 2>/dev/null || true' EXIT
sleep 1

# distro | image | GTK 4 and the tools the check uses
checks=(
  "Arch|archlinux:latest|pacman -Syu --noconfirm --needed gtk4 shared-mime-info xdg-utils desktop-file-utils diffutils curl >/dev/null"
  "Ubuntu 24.04|ubuntu:24.04|apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends libgtk-4-1 libgtk-4-bin shared-mime-info xdg-utils desktop-file-utils curl ca-certificates >/dev/null"
  "Fedora|fedora:latest|dnf install -y -q gtk4 shared-mime-info xdg-utils desktop-file-utils diffutils curl tar gzip util-linux >/dev/null"
)
pids=() names=()
for c in "${checks[@]}"; do
  IFS='|' read -r distro image prep <<<"$c"
  [ -n "${ONLY:-}" ] && [[ "$distro" != "$ONLY"* ]] && continue
  docker run --rm --network host -v "$repo/$site:/site:ro" "$image" sh -c "
    set -e
    $prep
    useradd -m tester
    su tester -c 'sh /site/run.sh $url $((5 + ${#pids[@]}))'
  " > "$work/${distro%% *}.out" 2>&1 &
  pids+=($!) names+=("$distro")
done
fail=0 total=0
for i in "${!pids[@]}"; do
  distro=${names[$i]} out="$work/${names[$i]%% *}.out"
  status=0; wait "${pids[$i]}" || status=$?
  oks=$(grep -c '^ok ' "$out" || true) fails=$(grep -c '^FAIL ' "$out" || true)
  total=$((total + oks))
  if [ "$status" = 0 ] && [ "$fails" = 0 ] && [ "$oks" -gt 0 ] && grep -q '^run.sh: finished$' "$out"; then
    echo "ok   $distro: $oks checks ($(for a in $APPS; do printf '%s %s -> %s, ' "$a" "${OLD[$a]}" "${NEW[$a]}"; done | sed 's/, $//'))"
    [ -n "${VERBOSE:-}" ] && sed 's/^/     /' "$out"
  else
    echo "FAIL $distro (exit $status, $fails failed):"
    grep -v '^ok ' "$out" | sed 's/^/     /' | tail -n 80
    fail=1
  fi
done
[ $fail = 0 ] && echo "check_update: OK ($total checks)" || { echo "check_update: FAILED"; exit 1; }
