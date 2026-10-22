#!/usr/bin/env bash
# Requires a Mac with Speech permission already granted and fr-CA/en-US installed.
set -euo pipefail
cd "$(dirname "$0")/.."
[[ $(uname -s) == Darwin ]] || { echo 'IGNORÉ : test audio réel — Mac seulement.'; exit 0; }
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
clang -fobjc-arc -Wall -Wextra -Werror -framework Cocoa -framework AVFoundation -framework Speech \
  app/native/dictation-fixture-test.m -o "$work/dictation-fixture-test"
"$work/dictation-fixture-test" "$PWD/crates/wrapper/tests/fixtures/dictation-en.wav" "$PWD/crates/wrapper/tests/fixtures/dictation-fr.wav"
