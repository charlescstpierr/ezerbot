#!/usr/bin/env python3
import argparse
import os
from pathlib import Path
import shutil
import sys
import tempfile


PAYLOAD = {
    "ezerbot-linux": 0o755,
    "grokbot-wrapper": 0o755,
    "ezerbot.png": 0o644,
    "Ouvrir Ezerbot.sh": 0o755,
    "install-app-linux.py": 0o644,
    "INSTALLATION.md": 0o644,
}


def desktop_value(value):
    return str(value).replace("\\", "\\\\").replace("\n", "\\n").replace("\r", "\\r").replace("\t", "\\t")


def exec_argument(value):
    value = str(value).replace("%", "%%")
    for character in ("\\", '"', "`", "$"):
        value = value.replace(character, "\\" + character)
    return desktop_value('"' + value + '"')


def install_bundle(bundle, destination, data_home):
    for name, mode in PAYLOAD.items():
        source = bundle / name
        if not source.is_file():
            raise ValueError(f"Bundle incomplet : {source}")
        if mode == 0o755 and not os.access(source, os.X_OK):
            raise ValueError(f"Fichier non exécutable : {source}")

    destination.mkdir(parents=True, exist_ok=True)
    if bundle != destination:
        with tempfile.TemporaryDirectory(prefix=".install-", dir=destination) as temporary:
            staging = Path(temporary)
            for name, mode in PAYLOAD.items():
                shutil.copyfile(bundle / name, staging / name)
                (staging / name).chmod(mode)
            for name in PAYLOAD:
                os.replace(staging / name, destination / name)

    desktop = data_home / "applications/ezerbot.desktop"
    desktop.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".ezerbot-", dir=desktop.parent) as temporary:
        staged_desktop = Path(temporary) / desktop.name
        staged_desktop.write_text(
            "[Desktop Entry]\nType=Application\nName=Ezerbot\n"
            "Comment=Conversations locales avec Ezerbot\n"
            # GIO checks argv[0] before expanding %% in Exec paths.
            f"Exec=/usr/bin/env bash {exec_argument(destination / 'Ouvrir Ezerbot.sh')}\n"
            f"Icon={desktop_value(destination / 'ezerbot.png')}\n"
            "Terminal=false\nCategories=Utility;\nStartupWMClass=Ezerbot\n",
            encoding="utf-8",
        )
        staged_desktop.chmod(0o644)
        os.replace(staged_desktop, desktop)
    return desktop


def main():
    parser = argparse.ArgumentParser(description="Installer ce bundle Ezerbot pour l’utilisateur courant, sans Rust ni sudo.")
    parser.add_argument("--install-dir", type=Path, default=Path.home() / ".local/opt/ezerbot",
                        help="Dossier de l’application. Par défaut : ~/.local/opt/ezerbot.")
    options = parser.parse_args()
    if sys.platform != "linux":
        parser.error("Ce bundle s’installe sous Linux.")
    data_home = Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share")
    if not data_home.is_absolute():
        data_home = Path.home() / ".local/share"
    destination = options.install_dir.expanduser().resolve()
    try:
        desktop = install_bundle(Path(__file__).resolve().parent, destination, data_home)
    except (OSError, ValueError) as error:
        parser.exit(1, f"Installation impossible : {error}\n")
    print(f"Application installée : {destination}\nRaccourci : {desktop}")


if __name__ == "__main__":
    main()
