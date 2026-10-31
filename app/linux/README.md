# Ouvrir Ezerbot sous Linux

Installez Rust avec rustup. Sur Debian ou Ubuntu, installez les bibliothèques système.

```sh
sudo apt install build-essential pkg-config libgtk-3-dev libwebkit2gtk-4.1-dev
tools/build-app-linux.sh
"app/build/linux/Ouvrir Ezerbot.sh"
```

Le script produit les deux exécutables, une icône et `ezerbot.desktop` dans `app/build/linux`.
Le lanceur et le fichier desktop utilisent les exécutables de ce dossier.
Après avoir déplacé le dépôt, relancez le script pour actualiser les chemins du fichier desktop.
Le script respecte `CARGO` et le dossier de compilation effectif de Cargo.
`EZERBOT_BUILD_DEBUG=1` produit une version de développement.

Les conversations et les notations restent dans le dossier de données XDG d’Ezerbot.
L’option `--data-dir /chemin/isolé` permet un lancement avec des données séparées.
Le menu **Quitter Ezerbot**, Ctrl+Q et la fermeture de la fenêtre arrêtent aussi le wrapper.

## Fonctions natives non portées

Ces fonctions restent indisponibles sous Linux et leurs commandes sont masquées.
L’inventaire de #598 renvoie vers les mêmes tickets.

| Fonction | Travail restant | Suivi |
| --- | --- | --- |
| Dictée | Reconnaissance vocale native | #606, #607 |
| Microphone et mémos vocaux | Sélection et capture audio | #606, #607 |
| Appels et mode voix | Capture et lecture audio natives | #607 |
| Enregistrement local de réunion | Capture audio locale | #606, #607 |
| Capture de l’écran | Permissions et services du bureau | #608 |
| Contrôle de l’écran | Actions d’entrée par le portail RemoteDesktop | Optionnel dans #608, à cadrer |
| Démonstrations | Capture des gestes et de l’écran | Dépend de #608 |
| Fenêtre flottante | Fenêtre et raccourci global Linux | #609 |

Le bac à sable YOLO est suivi dans #528.
Les services de #598 comprennent Secret Service, les notifications D-Bus,
les liens profonds et l’ouverture par `xdg-open`.

## Preuve sans compte réel

Construisez avec `EZERBOT_BUILD_DEBUG=1`, puis lancez avec `EZERBOT_FAKE_SIGNIN=1`
et un `--data-dir` temporaire. Le faux fournisseur local répond `OK`.
Il existe uniquement dans le wrapper de développement.
Les preuves de la coque doivent utiliser sa vraie fenêtre sous Xvfb.
Sous Xvfb, utilisez `WEBKIT_DISABLE_DMABUF_RENDERER=1` pour permettre le rendu
de WebKit sans export de buffers graphiques. Le lanceur ne force pas cette option.

La CI de #595 doit installer `libgtk-3-dev` et `libwebkit2gtk-4.1-dev`
avant de compiler le workspace qui comprend maintenant cette coque.
Le contrôle **Coque Linux** installe ces dépendances et vérifie la coque séparément.
