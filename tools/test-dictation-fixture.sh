#!/usr/bin/env bash
# Real on-device dictation test (en-US then fr-CA) on a Mac with both locales installed.
# The test binary lives at ONE fixed path shared by every worktree, ad hoc signed with a fixed
# identifier, and is rebuilt only when what it is built from changes: macOS ties the Speech
# permission to that exact signed binary (its cdhash), so it is granted once.
#   tools/test-dictation-fixture.sh            install if needed, then run (asks for Speech once if needed)
#   tools/test-dictation-fixture.sh --install  install if needed, run nothing
#   tools/test-dictation-fixture.sh --status   install if needed, print the Speech status, never ask
set -euo pipefail
[[ $(uname -s) == Darwin ]] || { echo 'ÉCHEC : test audio réel, Mac seulement.' >&2; exit 1; }
mode=${1:-run}
case $mode in run | --install | --status) ;; *) echo "usage : $0 [--install|--status]" >&2; exit 2 ;; esac
# Under $HOME, outside every worktree, outside TCC-protected folders (Desktop, Documents) and caches.
dir="$HOME/.local/share/ezerbot/dictation-fixture-test"

# One install or run at a time across worktrees (so one prompt at most). This very process holds the
# lock: it opens $dir/.lock on descriptor 9, and a short perl child takes flock(2) LOCK_EX on that
# descriptor. A flock lock belongs to the open file description, so it stays held while this shell, or
# anything it started (clang, the test binary), keeps descriptor 9 open, and the kernel frees it when the
# last one is gone, even on SIGKILL (flock(2) NOTES: dup(2)/fork(2) copies are "multiple references to a
# single lock"). No supervisor stands between the lock and the work: the former `lockf FILE COMMAND` form
# held the lock in the lockf process alone, and killing it freed the lock while the script ran on.
# lockf(1) uses the same flock(2) on the same file, so both forms exclude each other. /usr/bin/perl ships
# with every macOS. A holder runs at most about 2 min (45 s prompt + 2 x 30 s); a waiter gives up after
# 180 s with exit 75 (EX_TEMPFAIL; our own codes are 0, 1, 2 and the binary's).
wait_s=180
mkdir -p "$dir"
exec 9>>"$dir/.lock"
status=0
/usr/bin/perl -MFcntl=:flock -e '
  $SIG{ALRM} = sub { exit 75 };
  open(my $fh, ">>&=", 9) or exit 1;
  alarm $ARGV[0];
  flock($fh, LOCK_EX) or exit 1;
  alarm 0;' "$wait_s" || status=$?
case $status in
  0) ;;
  75) echo "ÉCHEC : $dir/.lock tenu plus de $wait_s s par un autre test de dictée." >&2; exit 75 ;;
  *) echo "ÉCHEC : impossible de prendre le verrou $dir/.lock (perl : code $status)." >&2; exit 1 ;;
esac

cd "$(dirname "$0")/.."
bin="$dir/ezerbot-dictation-fixture-test"
id=com.ezerbot.dictation-fixture-test
src=app/native/dictation-fixture-test.m
plist=app/native/dictation-fixture-test-Info.plist
cflags=(-fobjc-arc -Wall -Wextra -Werror -framework Cocoa -framework AVFoundation -framework CoreAudio
  -framework AudioToolbox -framework Speech -sectcreate __TEXT __info_plist "$plist")

# Contents of the local sources (as clang sees them), the plist and the recipe; never paths or dates.
fingerprint() {
  {
    printf '%s\n' "${cflags[*]}" "codesign --identifier $id"
    for file in $(clang -MM -MT x -fobjc-arc "$src" | tr -d '\134' | cut -d: -f2-) "$plist"; do
      printf '%s %s\n' "$file" "$(shasum -a 256 <"$file")"
    done
  } | shasum -a 256 | cut -d' ' -f1
}

cdhash() { codesign -dvvv "$1" 2>&1 | sed -n 's/^CDHash=//p' || true; }

work=
trap '[[ -z $work ]] || rm -rf "${work:?}"' EXIT
want=$(fingerprint)
# A rebuild changes the cdhash, so macOS asks for the permission again: rebuild only on a real change.
# The record is "<fingerprint> <cdhash of the binary built from it>": it is trusted only if that
# cdhash is the installed binary's, and it is removed before the binary is replaced, so a run that
# dies between the two renames leaves no record (rebuild), never another binary's record.
if [[ ! -f $dir/fingerprint || $(<"$dir/fingerprint") != "$want $(cdhash "$bin")" ]] || ! codesign --verify --strict "$bin" 2>/dev/null; then
  rm -rf "${dir:?}"/.build.* # left by a run that died mid-build; we hold the lock
  work=$(mktemp -d "$dir/.build.XXXXXX")
  clang "${cflags[@]}" "$src" -o "$work/bin"
  codesign --force --sign - --identifier "$id" "$work/bin"
  codesign --verify --strict "$work/bin"
  built=$(cdhash "$work/bin")
  [[ $built =~ ^[0-9a-f]{40}$ ]] || { echo "ÉCHEC : CDHash illisible pour $work/bin." >&2; exit 1; }
  printf '%s %s\n' "$want" "$built" >"$work/fingerprint"
  rm -f "${dir:?}/fingerprint"
  mv -f "$work/bin" "$bin" # rename(2) in one directory: a reader sees the old or the new binary
  mv -f "$work/fingerprint" "$dir/fingerprint"
  echo "Binaire de dictée installé : $bin (CDHash=$built)" >&2
fi

case $mode in
  --install) exit 0 ;;
  --status) "$bin" --status ;;
  run) "$bin" "$PWD/crates/wrapper/tests/fixtures/dictation-en.wav" "$PWD/crates/wrapper/tests/fixtures/dictation-fr.wav" ;;
esac
