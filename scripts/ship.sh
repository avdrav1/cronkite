#!/usr/bin/env bash
# Release pipeline. Each step is separate so the ship-it skill can confirm
# with the user before anything leaves the machine. APP is brick (the default), slate,
# frame, or etch.
#
#   scripts/ship.sh [APP] preflight          checks only, changes nothing
#   scripts/ship.sh [APP] bump patch|minor|major|X.Y.Z
#   scripts/ship.sh [APP] build              release binary (built in Ubuntu 24.04) + tarball + latest.json in dist/
#   scripts/ship.sh [APP] build --macos      the macOS app (arm64): on a Mac, scripts/mac_bundle.sh here;
#                                            elsewhere release-macos.yml on GitHub at this commit
#                                            (pushed), waited for, then fetch-macos
#   scripts/ship.sh [APP] fetch-macos RUN_ID release-macos.yml run RUN_ID's DMG and tarball -> dist/
#   scripts/ship.sh [APP] build --windows    the Windows installer (x86_64): in MSYS2's UCRT64 shell,
#                                            scripts/win_bundle.sh here; elsewhere release-windows.yml
#                                            on GitHub at this commit (pushed), waited for, then
#                                            fetch-windows
#   scripts/ship.sh [APP] fetch-windows RUN_ID
#                                            release-windows.yml run RUN_ID's Setup.exe -> dist/
#   scripts/ship.sh [APP] verify             the binary starts on clean Arch, Ubuntu 24.04, and Fedora; the
#                                            installer works there; latest.json matches dist/
#   scripts/ship.sh [APP] upload [--dry-run] tarballs, DMG, Setup.exe, checksums, latest.json and its
#                                            signature -> Cloudflare R2
#   scripts/ship.sh [APP] site   [--dry-run] the app's download page and installers (sites/APP) -> Cloudflare Pages
#   scripts/ship.sh suite-site [--dry-run]   suite page and the suite's installers (sites/cronkite) -> Cloudflare Pages
#   scripts/ship.sh video [--dry-run]        the explainer's English cuts (dist/video/, from
#                                            video/render.mjs) and a poster -> R2 video/ID/
#   scripts/ship.sh [APP] tag                annotated git tag (local only): vX.Y.Z for Brick,
#                                            APP-vX.Y.Z for the others (slate-v0.3.0)
#
# Brick's release sits at the bucket's root (latest.json, releases/), the other apps' under
# their name (slate/, frame/, etch/). A release is the Linux tarball and, when built, the
# macOS tarball and DMG and the Windows Setup.exe beside it; latest.json lists each platform
# (scripts/release_json.py), and latest.json.sig is its Ed25519 signature, made with the
# private key at CRONKITE_RELEASE_KEY, which lives outside the repository (ADR 0012;
# scripts/release_key.py). Without it latest.json is written unsigned, and verify and
# upload refuse it.
set -euo pipefail
cd "$(dirname "$0")/.."

[ -f .env.ship ] && set -a && . ./.env.ship && set +a
APP=brick
case "${1:-}" in brick|slate|frame|etch) APP=$1; shift ;; esac
TARGET=x86_64-unknown-linux-gnu
case $APP in
brick)
  ID=dev.cronkite.Brick  # desktop entry and icon names, as the app ID (APP_ID in apps/brick/src/main.rs)
  MANIFEST=Cargo.toml; SECTION=workspace.package  # Brick's version is the workspace's
  PREFIX=""; PAGES_VAR=CF_PAGES_PROJECT; TAG_PREFIX=v
  INSTALL_CHECK=scripts/check_install.sh
  ;;
slate)
  ID=dev.cronkite.Slate  # APP_ID in apps/slate/src/main.rs
  MANIFEST=apps/slate/Cargo.toml; SECTION=package
  PREFIX="slate/"; PAGES_VAR=CF_SLATE_PAGES_PROJECT; TAG_PREFIX=slate-v
  INSTALL_CHECK=scripts/check_install_slate.sh
  ;;
frame)
  ID=dev.cronkite.Frame  # APP_ID in apps/frame/src/main.rs
  MANIFEST=apps/frame/Cargo.toml; SECTION=package
  PREFIX="frame/"; PAGES_VAR=CF_FRAME_PAGES_PROJECT; TAG_PREFIX=frame-v
  INSTALL_CHECK=scripts/check_install_frame.sh
  ;;
etch)
  ID=dev.cronkite.Etch  # APP_ID in apps/etch/src/main.rs
  MANIFEST=apps/etch/Cargo.toml; SECTION=package
  PREFIX="etch/"; PAGES_VAR=CF_ETCH_PAGES_PROJECT; TAG_PREFIX=etch-v
  INSTALL_CHECK=scripts/check_install_etch.sh
  ;;
esac
# Where the build puts latest.json and the deploy its copy of the site: dist/ for Brick,
# dist/APP/ for the others.
OUT="dist${PREFIX:+/${PREFIX%/}}"
version() { sed -n "/^\[$SECTION\]/,/^\[/{s/^version = \"\(.*\)\"/\1/p;}" "$MANIFEST"; }
V=$(version)
TAG="$TAG_PREFIX$V"
NAME="$APP-$V-x86_64-linux"
TAR="dist/$NAME.tar.gz"
# The macOS build (MAC-7): scripts/mac_bundle.sh's names, Apple Silicon only.
MAC_NAME="$APP-$V-arm64-macos"
MAC_TAR="dist/$MAC_NAME.tar.gz"
MAC_DMG="dist/$MAC_NAME.dmg"
# The Windows build (WIN-5, WIN-6): scripts/win_bundle.sh's installer, x86_64.
WIN_NAME="$APP-$V-x86_64-windows-setup.exe"
WIN_SETUP="dist/$WIN_NAME"
DRY=false; [ "${2:-}" = "--dry-run" ] && DRY=true
BUILD_IMAGE=brick-build:ubuntu-24.04
need() { for v in "$@"; do [ -n "${!v:-}" ] || { echo "missing env: $v (see .env.ship.example)"; exit 1; }; done; }
run() { if $DRY; then echo "[dry-run] $*"; else "$@"; fi; }
wr() { npx --yes wrangler@4 "$@"; }
# Where the app's latest.json and releases/ are.
base() { echo "$DOWNLOAD_BASE_URL${PREFIX:+/${PREFIX%/}}"; }
sha256_of() { python3 -c 'import hashlib, sys; print(hashlib.sha256(open(sys.argv[1], "rb").read()).hexdigest())' "$1"; }
# Write $OUT/latest.json from the release files in dist/ (scripts/release_json.py has its
# shape: each platform under "platforms", then Linux's fields at the top level, last), and
# $OUT/latest.json.sig when CRONKITE_RELEASE_KEY is set. Needs the Linux tarball: until it's
# built, nothing is written.
latest() {
  need DOWNLOAD_BASE_URL
  if [ ! -f "$TAR" ]; then echo "latest.json waits for the Linux tarball (scripts/ship.sh $APP build)"; return 0; fi
  mkdir -p "$OUT"
  python3 scripts/release_json.py write "$APP" "$(base)/releases/v$V" "$V" "$TAR" "$MAC_TAR" "$MAC_DMG" "$WIN_SETUP" "$OUT/latest.json"
  echo "$OUT/latest.json: $APP $V for $(python3 -c 'import json, sys; print(", ".join(json.load(open(sys.argv[1]))["platforms"]))' "$OUT/latest.json")$([ -f "$OUT/latest.json.sig" ] && echo ", signed")"
}
# Build on GitHub: run release-$1.yml (macos or windows) for APP at HEAD, which must be on
# GitHub already (pushing is the user's call), wait for it, then fetch-$1 its artifact.
on_github() {
  branch=$(git branch --show-current); sha=$(git rev-parse HEAD)
  if [ -z "$branch" ] || [ -n "$(git status --porcelain)" ] ||
    [ "$(git rev-parse -q --verify "origin/$branch" || true)" != "$sha" ]; then
    echo "the $1 build runs on GitHub at origin/${branch:-BRANCH}: commit, then push $sha first (ask before pushing)"
    exit 1
  fi
  since=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  gh workflow run "release-$1.yml" --ref "$branch" -f apps="$APP"
  id=""
  for _ in $(seq 30); do
    sleep 4
    id=$(gh run list --workflow "release-$1.yml" --branch "$branch" --event workflow_dispatch --limit 10 \
      --json databaseId,headSha,createdAt \
      --jq "map(select(.headSha == \"$sha\" and .createdAt >= \"$since\")) | .[-1].databaseId // empty")
    [ -n "$id" ] && break
  done
  [ -n "$id" ] || { echo "release-$1.yml didn't start on $branch"; exit 1; }
  echo "release-$1.yml run $id: $(gh run view "$id" --json url --jq .url)"
  gh run watch "$id" --exit-status >/dev/null || { echo "run $id failed: gh run view $id --log-failed"; exit 1; }
  exec "$0" "$APP" "fetch-$1" "$id"
}
# fetch_run KIND RUN_ID FILE...: a green release-KIND.yml run's artifact KIND-APP (it built
# at HEAD, and its verify job installed and started the app on a clean runner), whose FILEs
# (names in dist/, so this version's) and their .sha256 files go into dist/; then latest.json.
fetch_run() {
  kind=$1 id=$2; shift 2
  read -r conclusion head <<<"$(gh run view "$id" --json conclusion,headSha --jq '"\(.conclusion) \(.headSha)"')"
  [ "$conclusion" = success ] || { echo "run $id: ${conclusion:-still running}; only a green run ships"; exit 1; }
  [ "$head" = "$(git rev-parse HEAD)" ] || { echo "run $id built $head, not HEAD ($(git rev-parse HEAD))"; exit 1; }
  tmp="dist/.$kind-$id"; rm -rf "$tmp"; mkdir -p dist
  gh run download "$id" -n "$kind-$APP" -D "$tmp"
  for f in "$@"; do
    [ -f "$tmp/$f" ] || { echo "run $id has no $f (not $APP $V?): $(ls "$tmp" | tr '\n' ' ')"; exit 1; }
    [ "$(sha256_of "$tmp/$f")" = "$(cut -d' ' -f1 "$tmp/$f.sha256")" ] || { echo "$f doesn't match its sha256"; exit 1; }
    mv -f "$tmp/$f" "$tmp/$f.sha256" dist/
    echo "fetched dist/$f ($(wc -c < "dist/$f" | tr -d ' ') bytes, sha256 $(cut -d' ' -f1 "dist/$f.sha256"))"
  done
  rm -rf "$tmp"
  latest
}
# Create Pages project $1 unless it exists. Match the name exactly: the list's domains
# column holds brick.cronkite.cc.
pages_project() {
  if ! wr pages project list --json | python3 -c 'import json, sys
sys.exit(sys.argv[1] not in [p["Project Name"] for p in json.load(sys.stdin)])' "$1"; then
    # --force: wrangler 4.149 otherwise hands new projects to Workers and fails on a plain
    # folder of assets. Like `spreadsheet`, this is a classic Pages project.
    run wr pages project create "$1" --production-branch main --force
  fi
}

case "${1:-}" in
preflight)
  fail=0
  [ -z "$(git status --porcelain)" ] || { echo "FAIL working tree not clean"; fail=1; }
  b=$(git branch --show-current); [ "$b" = main ] || { echo "FAIL on branch $b, not main"; fail=1; }
  git rev-parse "$TAG" >/dev/null 2>&1 && { echo "FAIL tag $TAG already exists; bump first"; fail=1; }
  for f in packaging/$ID.desktop packaging/$ID.svg sites/$APP/index.html sites/$APP/install.sh sites/$APP/install.ps1 "$INSTALL_CHECK"; do
    [ -f "$f" ] || { echo "FAIL $f missing: $APP has no packaging, download page, or install check yet"; fail=1; }
  done
  cargo fmt --all --check || { echo "FAIL fmt"; fail=1; }
  cargo clippy --workspace --all-targets --all-features -- -D warnings || { echo "FAIL clippy"; fail=1; }
  cargo test --workspace || { echo "FAIL tests"; fail=1; }
  command -v npx >/dev/null || { echo "FAIL npx not found (needed for wrangler)"; fail=1; }
  docker info >/dev/null 2>&1 || { echo "FAIL docker not usable (the release builds in a container)"; fail=1; }
  for v in CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID R2_BUCKET "$PAGES_VAR" DOWNLOAD_BASE_URL; do
    [ -n "${!v:-}" ] || { echo "FAIL env $v not set"; fail=1; }
  done
  echo "$APP version $V"; python3 scripts/backlog.py status
  [ $fail = 0 ] && echo "PREFLIGHT OK" || { echo "PREFLIGHT FAILED"; exit 1; }
  ;;
bump)
  new="${2:?usage: bump patch|minor|major|X.Y.Z}"
  IFS=. read -r ma mi pa <<<"$V"
  case "$new" in
    patch) new="$ma.$mi.$((pa+1))" ;; minor) new="$ma.$((mi+1)).0" ;; major) new="$((ma+1)).0.0" ;;
  esac
  sed -i "/^\[$SECTION\]/,/^\[/s/^version = \".*\"/version = \"$new\"/" "$MANIFEST"
  cargo check -q --workspace
  echo "$APP $V -> $new"
  ;;
build)
  if [ "${2:-}" = --macos ]; then
    if [ "$(uname -s)" = Darwin ]; then
      # On an arm64 Mac with Homebrew's GTK (release-macos.yml runs this on macos-14).
      cargo build --release --locked -p $APP
      scripts/mac_bundle.sh $APP target/release dist
      if [ -n "${DOWNLOAD_BASE_URL:-}" ]; then latest; fi
      exit 0
    fi
    on_github macos
  fi
  if [ "${2:-}" = --windows ]; then
    if [ "${MSYSTEM:-}" = UCRT64 ]; then
      # In MSYS2's UCRT64 shell with its GTK and Inno Setup 6 (release-windows.yml runs this
      # on windows-latest).
      cargo build --release --locked -p $APP
      scripts/win_bundle.sh $APP target/release dist
      if [ -n "${DOWNLOAD_BASE_URL:-}" ]; then latest; fi
      exit 0
    fi
    on_github windows
  fi
  need DOWNLOAD_BASE_URL
  # Built in Ubuntu 24.04 so it runs on that and anything newer (PKG-3); its own target
  # dir, so the host's builds stay as they are.
  docker build -q -t "$BUILD_IMAGE" -f packaging/build.Dockerfile packaging >/dev/null
  docker run --rm -u "$(id -u):$(id -g)" -v "$PWD:/src" -w /src \
    -e CARGO_HOME=/src/target/container/cargo -e CARGO_TARGET_DIR=/src/target/container \
    "$BUILD_IMAGE" cargo build --release --locked -p $APP --target $TARGET
  rm -rf "dist/$NAME"; mkdir -p "dist/$NAME"
  cp "target/container/$TARGET/release/$APP" "dist/$NAME/"
  strip "dist/$NAME/$APP" || true
  cp packaging/$ID.desktop packaging/$ID.svg "dist/$NAME/"
  [ -f apps/$APP/README.md ] && cp apps/$APP/README.md "dist/$NAME/"
  cp LICENSE* "dist/$NAME/" 2>/dev/null || true
  # The Rust crates compiled into the binary, their licences and texts (PKG-6); fails on a
  # crate without a licence text or outside the allowlist.
  python3 scripts/rust_licenses.py $APP $TARGET "dist/$NAME/THIRD-PARTY-RUST.txt"
  tar -C dist -czf "$TAR" "$NAME"
  (cd dist && sha256sum "$NAME.tar.gz" > "$NAME.tar.gz.sha256")
  sha=$(cut -d' ' -f1 "$TAR.sha256"); size=$(stat -c%s "$TAR")
  echo "built $TAR ($size bytes, sha256 $sha)"
  latest
  ;;
fetch-macos)
  id="${2:?usage: scripts/ship.sh APP fetch-macos RUN_ID (a release-macos.yml run)}"
  fetch_run macos "$id" "$MAC_NAME.tar.gz" "$MAC_NAME.dmg"
  ;;
fetch-windows)
  id="${2:?usage: scripts/ship.sh APP fetch-windows RUN_ID (a release-windows.yml run)}"
  fetch_run windows "$id" "$WIN_NAME"
  ;;
verify)
  [ -x "dist/$NAME/$APP" ] || { echo "run build first"; exit 1; }
  scripts/check_release.sh "dist/$NAME/$APP"
  "$INSTALL_CHECK" "$TAR"
  # The macOS and Windows builds were checked on their runners (release-*.yml's verify
  # jobs); here, that latest.json names this version's files in dist/ as they are.
  python3 scripts/release_json.py check "$OUT/latest.json" dist
  ;;
upload)
  need CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID R2_BUCKET DOWNLOAD_BASE_URL
  [ -f "$TAR" ] || { echo "run build first"; exit 1; }
  # Signed and matching dist/ before anything leaves the machine.
  latest
  python3 scripts/release_json.py check "$OUT/latest.json" dist
  run wr r2 object put "$R2_BUCKET/${PREFIX}releases/v$V/$NAME.tar.gz" --file "$TAR" --content-type application/gzip --remote
  run wr r2 object put "$R2_BUCKET/${PREFIX}releases/v$V/$NAME.tar.gz.sha256" --file "$TAR.sha256" --content-type text/plain --remote
  if [ -f "$MAC_TAR" ] && [ -f "$MAC_DMG" ]; then
    run wr r2 object put "$R2_BUCKET/${PREFIX}releases/v$V/$MAC_NAME.tar.gz" --file "$MAC_TAR" --content-type application/gzip --remote
    run wr r2 object put "$R2_BUCKET/${PREFIX}releases/v$V/$MAC_NAME.tar.gz.sha256" --file "$MAC_TAR.sha256" --content-type text/plain --remote
    run wr r2 object put "$R2_BUCKET/${PREFIX}releases/v$V/$MAC_NAME.dmg" --file "$MAC_DMG" --content-type application/x-apple-diskimage --remote
    run wr r2 object put "$R2_BUCKET/${PREFIX}releases/v$V/$MAC_NAME.dmg.sha256" --file "$MAC_DMG.sha256" --content-type text/plain --remote
  else
    echo "no macOS build in dist/ ($MAC_NAME.tar.gz and .dmg; build --macos or fetch-macos)"
  fi
  if [ -f "$WIN_SETUP" ]; then
    run wr r2 object put "$R2_BUCKET/${PREFIX}releases/v$V/$WIN_NAME" --file "$WIN_SETUP" --content-type application/octet-stream --remote
    run wr r2 object put "$R2_BUCKET/${PREFIX}releases/v$V/$WIN_NAME.sha256" --file "$WIN_SETUP.sha256" --content-type text/plain --remote
  else
    echo "no Windows build in dist/ ($WIN_NAME; build --windows or fetch-windows)"
  fi
  # Last, so they never name a file that isn't there yet. A client that reads the new
  # latest.json beside the old signature refuses the pair and tries again at its next start.
  run wr r2 object put "$R2_BUCKET/${PREFIX}latest.json" --file "$OUT/latest.json" --content-type application/json --remote
  run wr r2 object put "$R2_BUCKET/${PREFIX}latest.json.sig" --file "$OUT/latest.json.sig" --content-type text/plain --remote
  echo "check: curl -fsSI $(base)/releases/v$V/$NAME.tar.gz"
  if [ -f "$MAC_DMG" ]; then echo "check: curl -fsSI $(base)/releases/v$V/$MAC_NAME.dmg"; fi
  if [ -f "$WIN_SETUP" ]; then echo "check: curl -fsSI $(base)/releases/v$V/$WIN_NAME"; fi
  ;;
site)
  need CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID "$PAGES_VAR"
  [ -f "$OUT/latest.json" ] || { echo "run build first"; exit 1; }
  rm -rf "$OUT/site"; cp -r "sites/$APP" "$OUT/site"
  # The page shows the release written into it, not a latest.json beside it: Pages fails
  # to publish a .json file in the slate project ("Unknown internal error", 2026-10-12).
  python3 -c 'import sys; p, rel = sys.argv[1], open(sys.argv[2]).read().strip()
html = open(p).read(); assert "@RELEASE_JSON@" in html, p
open(p, "w").write(html.replace("@RELEASE_JSON@", rel.replace("</", "<\\/")))' "$OUT/site/index.html" "$OUT/latest.json"
  # The installers read latest.json from the bucket (PKG-1; install.ps1, WIN-5).
  need DOWNLOAD_BASE_URL
  sed -i "s|@DOWNLOAD_BASE_URL@|$(base)|" "$OUT/site/install.sh" "$OUT/site/install.ps1"
  pages_project "${!PAGES_VAR}"
  run wr pages deploy "$OUT/site" --project-name "${!PAGES_VAR}" --branch main --commit-dirty=true
  ;;
suite-site)
  # cronkite.cc: static, no release data, so it deploys only when sites/cronkite changes.
  # The custom domain cronkite.cc is attached to the project once, in the dashboard.
  need CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID CF_SUITE_PAGES_PROJECT
  rm -rf dist/cronkite-site; cp -r sites/cronkite dist/cronkite-site
  # Browsers keep a screenshot for hours under its name (Pages: max-age=14400), so the page
  # names each one with its content's hash (shots/x.webp?v=1a2b3c4d): a retaken shot gets a
  # new address and shows at once.
  python3 -c 'import hashlib, os, re, sys
site = sys.argv[1]
page = os.path.join(site, "index.html")
html = open(page).read()
def stamp(m):
    data = open(os.path.join(site, m.group(1)), "rb").read()
    return m.group(1) + "?v=" + hashlib.sha256(data).hexdigest()[:8]
open(page, "w").write(re.sub(r"(shots/[A-Za-z0-9_.-]+\.webp)", stamp, html))' dist/cronkite-site
  # The suite's installers (PKG-7) run each app's from its own site (https://APP.cronkite.cc),
  # so they have nothing to fill in.
  for f in install.sh install.ps1; do
    [ -f "dist/cronkite-site/$f" ] || { echo "sites/cronkite/$f missing"; exit 1; }
  done
  echo "cronkite.cc: $(ls dist/cronkite-site | tr '\n' ' ')"
  pages_project "$CF_SUITE_PAGES_PROJECT"
  run wr pages deploy dist/cronkite-site --project-name "$CF_SUITE_PAGES_PROJECT" --branch main --commit-dirty=true
  ;;
video)
  # The explainer video (video/render.mjs and --cut short) and a poster from the short
  # cut's title card, under video/ID/ where ID is the two cuts' hash: a new render gets new
  # addresses, so a page linking the old ones never shows a mix. Captions are burned in.
  need CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID R2_BUCKET DOWNLOAD_BASE_URL
  full=dist/video/cronkite-explainer.mp4 short=dist/video/cronkite-explainer-short.mp4
  for f in "$full" "$short"; do
    [ -f "$f" ] || { echo "no $f (cd video && node render.mjs, and --cut short)"; exit 1; }
  done
  poster=dist/video/cronkite-explainer-short.jpg
  ffmpeg -hide_banner -loglevel error -y -ss 1 -i "$short" -frames:v 1 -q:v 3 "$poster"
  id=$(cat "$full" "$short" | sha256sum | cut -c1-12)
  run wr r2 object put "$R2_BUCKET/video/$id/$(basename "$full")" --file "$full" --content-type video/mp4 --remote
  run wr r2 object put "$R2_BUCKET/video/$id/$(basename "$short")" --file "$short" --content-type video/mp4 --remote
  run wr r2 object put "$R2_BUCKET/video/$id/$(basename "$poster")" --file "$poster" --content-type image/jpeg --remote
  echo "video base: $DOWNLOAD_BASE_URL/video/$id"
  ;;
tag)
  git tag -a "$TAG" -m "$TAG"
  echo "tagged $TAG locally; push with: git push origin main $TAG"
  ;;
*) sed -n '2,32p' "$0"; exit 1 ;;
esac
