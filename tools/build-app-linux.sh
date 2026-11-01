#!/usr/bin/env bash
set -euo pipefail
umask 077

project_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
fail() { printf 'build-app-linux: %s\n' "$*" >&2; exit 1; }
[[ $(uname -s) == Linux ]] || fail 'Ce lanceur se construit sous Linux.'
[[ $# == 0 ]] || fail 'Usage : tools/build-app-linux.sh'

cargo_bin=${CARGO:-}
if [[ -z $cargo_bin ]]; then
  if [[ -x $HOME/.cargo/bin/cargo ]]; then cargo_bin=$HOME/.cargo/bin/cargo
  else cargo_bin=$(command -v cargo || true); fi
fi
[[ -n $cargo_bin ]] || fail 'Rust est requis. Installez rustup puis relancez ce script.'
for tool in pkg-config python3; do
  command -v "$tool" >/dev/null || fail "Commande manquante : $tool."
done
missing=()
for library in gtk+-3.0 webkit2gtk-4.1; do
  pkg-config --exists "$library" || missing+=("$library")
done
if (( ${#missing[@]} )); then
  printf 'Bibliothèque manquante : %s\n' "${missing[@]}" >&2
  fail 'Debian/Ubuntu : sudo apt install build-essential pkg-config libgtk-3-dev libwebkit2gtk-4.1-dev'
fi

cd "$project_dir"
df -BG "$project_dir"
free=$(df -B1 "$project_dir" | awk 'NR == 2 {print $4}')
(( free >= 25 * 1024 * 1024 * 1024 )) || fail 'Au moins 25 Gio libres sont requis avant la compilation.'
export CARGO_BUILD_JOBS=3
export EZERBOT_GIT_COMMIT=$(git rev-parse HEAD)
export EZERBOT_GIT_COMMIT_UNIX=$(git log -1 --format=%ct)
profile=release
flags=(--release)
if [[ ${EZERBOT_BUILD_DEBUG:-0} == 1 ]]; then profile=debug; flags=(); fi
"$cargo_bin" build "${flags[@]}" --locked -p ezerbot-linux -p grokbot-wrapper
target_dir=$("$cargo_bin" metadata --locked --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
bundle="$project_dir/app/build/linux"
mkdir -p "$bundle"
install -m 755 "$target_dir/$profile/ezerbot-linux" "$bundle/ezerbot-linux"
install -m 755 "$target_dir/$profile/grokbot-wrapper" "$bundle/grokbot-wrapper"
install -m 644 "$project_dir/app/ui/icon-192.png" "$bundle/ezerbot.png"
cat > "$bundle/Ouvrir Ezerbot.sh" <<'LAUNCHER'
#!/usr/bin/env bash
set -euo pipefail
exec "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/ezerbot-linux" "$@"
LAUNCHER
chmod 755 "$bundle/Ouvrir Ezerbot.sh"
python3 - "$bundle" <<'PY'
from pathlib import Path
import sys
bundle = Path(sys.argv[1])
def desktop_value(value):
    return str(value).replace("\\", "\\\\").replace("\n", "\\n").replace("\r", "\\r").replace("\t", "\\t")
def exec_argument(value):
    value = str(value).replace("%", "%%")
    for character in ("\\", '"', "`", "$"):
        value = value.replace(character, "\\" + character)
    return desktop_value('"' + value + '"')
(bundle / "ezerbot.desktop").write_text(
    "[Desktop Entry]\nType=Application\nName=Ezerbot\n"
    "Comment=Conversations locales avec Ezerbot\n"
    f"Exec={exec_argument(bundle / 'ezerbot-linux')}\n"
    f"Icon={desktop_value(bundle / 'ezerbot.png')}\n"
    "Terminal=false\nCategories=Utility;\nStartupWMClass=Ezerbot\n",
    encoding="utf-8",
)
PY
chmod 644 "$bundle/ezerbot.desktop"
printf 'Application créée : %s\nLancer : "%s/Ouvrir Ezerbot.sh"\n' "$bundle" "$bundle"
