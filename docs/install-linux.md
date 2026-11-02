# Installer le bundle Linux d'Ezerbot

Utilisez un bundle déjà compilé pour l'architecture de votre machine et une distribution compatible avec celle de compilation. L'installation ne demande ni Rust, ni Cargo, ni droits administrateur. Cette tranche ne publie pas de binaire à télécharger.

## Préparer la session Linux

Les programmes de l'installateur sont Python 3, Bash et `/usr/bin/env` fourni par coreutils. La fenêtre demande une session graphique, GTK 3 et WebKitGTK 4.1. Sur Ubuntu 24.04, les paquets runtime correspondants sont `python3`, `bash`, `coreutils`, `libgtk-3-0t64` et `libwebkit2gtk-4.1-0`. Les paquets de développement et Rust servent à compiler, pas à installer le bundle.

Les fonctions natives demandent aussi un bus D-Bus de session, un service Secret Service tel que GNOME Keyring, et `xdg-open` fourni par `xdg-utils`. Le terminal YOLO demande bubblewrap et un noyau Linux 6.12 ou plus récent, comme décrit dans l'ADR sur le mode YOLO. L'installateur ne fournit pas ces services et ne les configure pas.

## Installer dans votre profil

1. Extrayez le bundle en conservant les permissions exécutables. Le dossier contient `ezerbot-linux`, `grokbot-wrapper`, `ezerbot.png`, `Ouvrir Ezerbot.sh`, `install-app-linux.py` et cette notice `INSTALLATION.md`.
2. Dans ce dossier, lancez l'installation.

   ```sh
   python3 ./install-app-linux.py
   ```

   L'application est copiée dans `~/.local/opt/ezerbot`. Le script affiche le dossier installé et le chemin du raccourci.

   Pour choisir un autre dossier, utilisez cette commande à la place.

   ```sh
   python3 ./install-app-linux.py --install-dir "$HOME/Applications/Ezerbot Linux"
   ```

3. Ouvrez **Ezerbot** depuis le menu des applications. Le raccourci est enregistré dans `$XDG_DATA_HOME/applications/ezerbot.desktop`, ou `~/.local/share/applications/ezerbot.desktop` si la variable est vide ou relative.

Après installation, vous pouvez supprimer le dossier extrait. Gardez le dossier installé à sa place. Pour changer sa destination, relancez l'installateur avec le nouveau `--install-dir`. Il réécrit le raccourci et laisse l'ancien dossier en place.

Pour réinstaller ou mettre à jour, quittez Ezerbot puis relancez l'installateur du nouveau bundle avec la même destination. Seuls les fichiers connus du bundle et le raccourci sont remplacés. Les conversations dans `$XDG_DATA_HOME/ezerbot`, le coffre et les autres fichiers ne sont pas supprimés.

Pour un lancement sans installation, exécutez `./Ouvrir\ Ezerbot.sh` dans le dossier extrait. Aucun raccourci lié au dossier de compilation n'est livré.

## Produire et vérifier un bundle

Depuis les sources, sur une machine disposant des dépendances de compilation, lancez `tools/build-app-linux.sh`. Le résultat est dans `app/build/linux`. Pour transférer ce dossier en conservant les permissions, archivez-le avec `tar -C app/build -czf ezerbot-linux.tar.gz linux`.

La preuve d'installation se relance depuis la racine du dépôt.

```sh
python3 -m unittest discover -s tools -p test_linux_bundle_install.py -v
```

Sous Linux, ce test demande aussi `gio` fourni par `libglib2.0-bin` et `desktop-file-validate` fourni par `desktop-file-utils`. Il exécute le vrai script de packaging avec Cargo, pkg-config et l'espace disque simulés. Les exécutables sont des fixtures. Le test déplace le bundle, supprime son checkout temporaire, installe et lance le raccourci via GIO dans un profil temporaire. Il vérifie aussi les chemins avec espaces, les caractères réservés et la réinstallation sans perte des données synthétiques.

Cette preuve ne compile ni ne lance une vraie coque GTK. Le workflow **Coque Linux** garde la preuve de compilation native. La vérification d'un binaire distribué sur une machine propre reste nécessaire.
