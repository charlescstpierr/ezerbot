# Ouvrir Ezerbot sous Linux

Installez Rust avec rustup. Sur Debian ou Ubuntu, installez les bibliothèques système.

```sh
sudo apt install build-essential pkg-config libgtk-3-dev libwebkit2gtk-4.1-dev xdg-utils
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
Relancer Ezerbot sur le même dossier de données montre la fenêtre déjà ouverte.
Les liens qui demandent une nouvelle fenêtre s’ouvrent dans le navigateur par défaut avec `xdg-open`.
Seules les pages web hors du wrapper sortent de la coque. Un échec du navigateur s’affiche dans une boîte de dialogue.

## Panneau flottant

Le menu **Ezerbot** propose **Ouvrir le panneau flottant**. Les réglages choisissent le raccourci global.
Le panneau reste au premier plan, sans décoration. On le déplace par son en-tête, et Échap le masque.
Il garde son brouillon tant qu’Ezerbot reste ouvert.

Sous X11, Ezerbot réserve la touche avec `XGrabKey`, sans surveiller le clavier.
Un raccourci déjà pris par une autre application est annoncé et l’ancien reste actif.
Sous Wayland, aucun raccourci global pour l'instant : aucune application ne peut y réserver une touche, et le portail `org.freedesktop.portal.GlobalShortcuts` n'est pas encore utilisé (issue #635). Les réglages l'annoncent, et le panneau s'ouvre depuis le menu Ezerbot.
Sans X11 ni Wayland, le raccourci est annoncé indisponible et le menu reste disponible.
Sous Wayland, le premier plan dépend du compositeur : il n’est pas vérifié sous Wayland.
Fermer la fenêtre principale quitte toujours Ezerbot, donc le raccourci s’arrête avec elle.
Le pont natif ne sert que la page principale. Aucune page ne se charge dans un cadre : la coque ignore nativement toute réponse qui n'est pas la page du cadre principal. Aucun script d'un autre document ne peut donc atteindre le pont.

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

`cargo test -p ezerbot-linux` construit le wrapper puis ouvre la vraie fenêtre dans son propre Xvfb.
`tests/window.rs` constate l’affichage, fait un échange avec le faux fournisseur et clique des liens.
Il vérifie aussi la vraie connexion jusqu’à `xdg-open` et la relance de l’application déjà ouverte.
Les tests du panneau lancent aussi `openbox` et un `dbus-daemon` privé avec un faux portail GlobalShortcuts.
Ils envoient le raccourci, vérifient `_NET_WM_STATE_ABOVE` et prennent la touche avec un second client X.
Le test échoue si `Xvfb`, `openbox` ou `dbus-daemon` manque, sauf le test du panneau sur GitHub Actions, qui annonce son saut sans `openbox`.
Ils ne couvrent que les builds de développement.
