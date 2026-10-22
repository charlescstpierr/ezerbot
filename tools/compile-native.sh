#!/bin/bash
# Compile the Cocoa shell the same way the Mac build does, without linking
# the Rust engine. The property error on id (lastPathComponent) is a compile
# failure, so -c is enough.
set -euo pipefail

if [[ "$(uname -s)" != Darwin ]]; then
  printf '%s\n' 'La compilation de la coque Cocoa se fait sur macOS.' >&2
  exit 1
fi

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$(mktemp "${TMPDIR:-/tmp}/ezerbot-shell.XXXXXX.o")"
trap 'rm -f "$out"' EXIT

clang -fobjc-arc -Wall -Wextra -Werror -mmacosx-version-min=12.3 \
  -framework Cocoa -framework WebKit -framework UniformTypeIdentifiers \
  -framework UserNotifications -framework AVFoundation -framework Speech \
  -framework ApplicationServices -framework Security -framework Carbon \
  -c "$root/app/native/main.m" -o "$out"

printf '%s\n' 'Coque Cocoa compilée.'
