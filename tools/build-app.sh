#!/bin/bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bundle="$project_dir/app/build/Ezerbot.app"
cargo_bin="${CARGO:-$(command -v cargo || true)}"
if [[ -z "$cargo_bin" && -x "$HOME/.cargo/bin/cargo" ]]; then
  cargo_bin="$HOME/.cargo/bin/cargo"
fi
if [[ -z "$cargo_bin" ]]; then
  printf '%s\n' 'Rust est requis. Installez rustup puis relancez tools/build-app.sh.' >&2
  exit 1
fi
if [[ "$(uname -s)" != Darwin ]]; then
  printf '%s\n' 'Ce bundle natif se construit sur macOS.' >&2
  exit 1
fi

# Le diagnostic peut être testé sans ouvrir de trousseau réel ni construire.
# Le remplacement de l’outil sert uniquement aux tests locaux du script.
security_tool="${EZERBOT_SECURITY_TOOL:-/usr/bin/security}"
if [[ -n "${EZERBOT_SIGNING_KEYCHAIN:-}" ]]; then
  identities="$("$security_tool" find-identity -p codesigning "$EZERBOT_SIGNING_KEYCHAIN" 2>/dev/null || true)"
else
  identities="$("$security_tool" find-identity -p codesigning 2>/dev/null || true)"
fi
identity_hash="$(printf '%s\n' "$identities" | /usr/bin/awk '/"Ezerbot Signature Locale"/ && length($2) == 40 && $2 ~ /^[[:xdigit:]]+$/ { if (hash == "") hash = $2 } END { print hash }')"
if [[ -n "$identity_hash" ]]; then
  printf '%s\n' 'Signature avec « Ezerbot Signature Locale » : identité fondée sur un certificat stable.'
else
  printf '%s\n' 'Signature ad hoc : aucune identité « Ezerbot Signature Locale » trouvée.' \
    'Les secrets restent dans le coffre fichier ; le trousseau attend une signature stable.' \
    'Pour préparer cette identité, lancez vous-même tools/creer-signature.sh.'
fi
if [[ "${1:-}" == '--signature-info' && $# == 1 ]]; then
  exit 0
fi
if [[ $# != 0 ]]; then
  printf '%s\n' 'Usage : tools/build-app.sh [--signature-info]' >&2
  exit 1
fi

cd "$project_dir"
# The chat bundle (app/ui/chat.js, chat.css) is committed; rebuild it when its toolchain is installed.
if [[ -d "$project_dir/app/web/node_modules" ]] && command -v npm >/dev/null; then
  npm --prefix "$project_dir/app/web" run build
fi
# Debug bundles are incapable of contacting the macOS notification service.
# GROKBOT_NOTIFICATION_FAKE=recording selects the recording adapter at runtime.
profile=release
native_flags=(-DEZERBOT_RELEASE=1 -O2)
if [[ "${EZERBOT_BUILD_DEBUG:-0}" == 1 ]]; then
  profile=debug
  native_flags=(-DEZERBOT_DEBUG=1 -O0 -g)
  "$cargo_bin" build --locked -p grokbot-wrapper
else
  "$cargo_bin" build --release --locked -p grokbot-wrapper
fi
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
clang -fobjc-arc -Wall -Wextra -Werror "${native_flags[@]}" -mmacosx-version-min=12.3 \
  -framework Cocoa -framework WebKit -framework UniformTypeIdentifiers -framework UserNotifications \
  "$project_dir/app/native/main.m" -o "$bundle/Contents/MacOS/Ezerbot"
cp "$project_dir/target/$profile/grokbot-wrapper" "$bundle/Contents/MacOS/grokbot-wrapper"

cat > "$bundle/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleExecutable</key><string>Ezerbot</string>
  <key>CFBundleDevelopmentRegion</key><string>fr</string>
  <key>CFBundleIdentifier</key><string>local.grokbot.workspace</string>
  <key>CFBundleName</key><string>Ezerbot</string>
  <key>CFBundleDisplayName</key><string>Ezerbot</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>CFBundleIconFile</key><string>Grokbot</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSMinimumSystemVersion</key><string>12.3</string>
  <key>NSAppTransportSecurity</key><dict>
    <key>NSAllowsLocalNetworking</key><true/>
  </dict>
</dict></plist>
PLIST

iconset="$project_dir/app/build/Grokbot.iconset"
mkdir -p "$iconset"
clang -fobjc-arc -Wall -Wextra -Werror -O2 -framework Cocoa \
  "$project_dir/app/native/icon.m" -o "$project_dir/app/build/draw-icon"
"$project_dir/app/build/draw-icon" "$iconset"
iconutil -c icns "$iconset" -o "$bundle/Contents/Resources/Grokbot.icns"
plutil -lint "$bundle/Contents/Info.plist"
# A rebuild may reuse a bundle generated before the native executable was renamed.
rm -f "$bundle/Contents/MacOS/Grokbot"
if [[ -n "$identity_hash" ]]; then
  sign_with_identity() {
    if [[ -n "${EZERBOT_SIGNING_KEYCHAIN:-}" ]]; then
      codesign --force --sign "$identity_hash" --keychain "$EZERBOT_SIGNING_KEYCHAIN" --timestamp=none "$@"
    else
      codesign --force --sign "$identity_hash" --timestamp=none "$@"
    fi
  }
  # Pin the certificate, not the changing code hash. Do not silently fall back
  # when an existing identity cannot sign: report the error, not a new identity.
  sign_with_identity --identifier local.ezerbot.wrapper \
    --requirements "designated => identifier \"local.ezerbot.wrapper\" and certificate leaf = H\"$identity_hash\"" \
    "$bundle/Contents/MacOS/grokbot-wrapper"
  sign_with_identity --identifier local.grokbot.workspace \
    --requirements "designated => identifier \"local.grokbot.workspace\" and certificate leaf = H\"$identity_hash\"" \
    "$bundle"
else
  codesign --force --sign - "$bundle/Contents/MacOS/grokbot-wrapper"
  codesign --force --sign - "$bundle"
fi
codesign --verify --deep --strict "$bundle"
# Remove only the obsolete generated bundle, after its replacement verifies successfully.
rm -rf "$project_dir/app/build/Grokbot.app"
printf '\n%s\n' "Application créée : $bundle"
printf '%s\n' "Ouvrir : open \"$bundle\""
