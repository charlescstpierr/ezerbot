#!/usr/bin/env bash
set -euo pipefail
umask 077

fail() { printf 'linux-captures: %s\n' "$*" >&2; exit 1; }
[[ $(uname -s) == Linux ]] || fail 'Linux is required.'
[[ $# == 1 && $1 =~ ^[a-fA-F0-9]{7,40}$ ]] || fail 'Usage: tools/linux-captures.sh <7-40 digit commit SHA>'
root=$(git -C "$(dirname -- "$0")/.." rev-parse --show-toplevel)
sha=$(git -C "$root" rev-parse --verify "$1^{commit}") || fail 'Commit does not exist or is ambiguous.'
unix=$(git -C "$root" show -s --format=%ct "$sha")
output="$root/artifacts/captures-linux"
cache="$root/target/linux-captures"
mkdir -p "$output" "$cache"
exec 9>"$output/.lock"
flock -n 9 || fail 'Another Linux capture run owns the capture cache and output lock.'

scratch=$(mktemp -d /tmp/ezcap.XXXXXX)
worktree="$scratch/source"
staging=''
backup=''
child=''
cleanup() {
  status=$?
  trap - EXIT INT TERM
  if [[ -n $child ]]; then
    kill -TERM -- "-$child" 2>/dev/null || true
    for ((i=0; i<100; i++)); do
      kill -0 -- "-$child" 2>/dev/null || break
      sleep 0.05
    done
    kill -KILL -- "-$child" 2>/dev/null || true
    wait "$child" 2>/dev/null || true
  fi
  if [[ -n $backup && -d $backup && ! -e "$output/$sha" ]]; then mv -- "$backup" "$output/$sha"; fi
  [[ -z $staging ]] || rm -rf -- "$staging"
  [[ -z $backup ]] || rm -rf -- "$backup"
  if [[ -e "$worktree/.git" ]]; then git -C "$root" worktree remove --force "$worktree" >/dev/null; fi
  rm -rf -- "$scratch"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
run() {
  local directory=$1
  shift
  (cd "$directory" && exec setsid "$@") &
  child=$!
  local status=0
  wait "$child" || status=$?
  if (( status == 0 )); then child=''; fi
  return "$status"
}
disk_guard() {
  local free
  df -BG /workspace
  free=$(df -B1 /workspace | awk 'NR == 2 { print $4 }')
  [[ $free =~ ^[0-9]+$ ]] || fail 'Cannot determine free space with df -B1 /workspace.'
  (( free >= 25 * 1024 * 1024 * 1024 )) || fail "Need at least 25 GiB free on /workspace; df reports ${free} bytes."
}

git -C "$root" worktree add --detach "$worktree" "$sha" >/dev/null
[[ $(git -C "$worktree" rev-parse HEAD) == "$sha" ]] || fail 'Detached worktree commit differs.'
if git -C "$worktree" symbolic-ref -q HEAD >/dev/null; then fail 'Capture worktree is not detached.'; fi
for required in tools/linux-captures.cjs app/ui/wrapper-chrome-fixture.cjs app/ui/local-api-client.cjs app/web/package-lock.json; do
  [[ -f "$worktree/$required" ]] || fail "Unsupported commit $sha; missing $required."
done

chrome=''
for candidate in google-chrome google-chrome-stable chromium chromium-browser; do
  if chrome=$(command -v "$candidate"); then break; fi
done
[[ -n $chrome && -x $chrome ]] || fail 'Chrome is required. Install google-chrome or chromium on PATH.'
chrome=$(realpath "$chrome")
command -v xvfb-run >/dev/null || fail 'xvfb-run is required.'

disk_guard
run "$worktree/app/web" npm ci --no-audit --no-fund
disk_guard
run "$worktree/app/web" npm run build
disk_guard
rm -f -- "$cache/debug/grokbot-wrapper"
run "$worktree" env CARGO_TARGET_DIR="$cache" CARGO_BUILD_JOBS=3 EZERBOT_GIT_COMMIT="$sha" EZERBOT_GIT_COMMIT_UNIX="$unix" cargo build -p grokbot-wrapper --locked
binary="$cache/debug/grokbot-wrapper"
[[ -x $binary ]] || fail 'Wrapper build did not produce the capture binary.'

staging=$(mktemp -d "$output/.staging-$sha.XXXXXX")
mkdir -p "$scratch/home" "$scratch/config" "$scratch/cache" "$scratch/data" "$scratch/runtime" "$scratch/tmp"
printf 'linux-captures: rendering %s through Chromium. Native Linux shell capture is unavailable.\n' "$sha"
run "$worktree" env -i PATH="$PATH" HOME="$scratch/home" XDG_CONFIG_HOME="$scratch/config" XDG_CACHE_HOME="$scratch/cache" XDG_DATA_HOME="$scratch/data" XDG_RUNTIME_DIR="$scratch/runtime" TMPDIR="$scratch/tmp" LANG=C.UTF-8 TZ=UTC CHROME_BIN="$chrome" xvfb-run -a -s '-screen 0 1440x900x24 -nolisten tcp' node --experimental-websocket "$worktree/tools/linux-captures.cjs" "$sha" "$binary" "$staging"
[[ -f "$staging/manifest.json" ]] || fail 'Capture driver did not complete its manifest.'
if [[ -e "$output/$sha" ]]; then
  backup="$output/.previous-$sha-$$"
  mv -- "$output/$sha" "$backup"
fi
mv -- "$staging" "$output/$sha"
staging=''
printf 'linux-captures: %s/manifest.json\n' "$output/$sha"
