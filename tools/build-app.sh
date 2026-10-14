#!/bin/bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bundle="$project_dir/app/build/Grokbot.app"
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

cd "$project_dir"
CARGO_TARGET_DIR="$project_dir/target" "$cargo_bin" build --release --locked -p grokbot-wrapper
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
clang -fobjc-arc -Wall -Wextra -Werror -O2 -mmacosx-version-min=12.3 \
  -framework Cocoa -framework WebKit -framework UniformTypeIdentifiers \
  "$project_dir/app/native/main.m" -o "$bundle/Contents/MacOS/Grokbot"
cp "$project_dir/target/release/grokbot-wrapper" "$bundle/Contents/MacOS/grokbot-wrapper"

cat > "$bundle/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleExecutable</key><string>Grokbot</string>
  <key>CFBundleIdentifier</key><string>local.grokbot.workspace</string>
  <key>CFBundleName</key><string>Grokbot</string>
  <key>CFBundleDisplayName</key><string>Grokbot</string>
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
codesign --force --sign - "$bundle/Contents/MacOS/grokbot-wrapper"
codesign --force --sign - "$bundle"
codesign --verify --deep --strict "$bundle"
printf '\n%s\n' "Application créée : $bundle"
printf '%s\n' "Ouvrir : open \"$bundle\""
