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

## Fonctions natives non portées

Ces fonctions restent indisponibles sous Linux et leurs commandes sont masquées.
L’inventaire de #598 renvoie vers les mêmes tickets.

| Fonction | Travail restant | Suivi |
| --- | --- | --- |
| Dictée | Reconnaissance vocale native | #606, #607 |
| Microphone et mémos vocaux | Sélection et capture audio | #606, #607 |
| Appels et mode voix | Capture et lecture audio natives | #607 |
| Enregistrement local de réunion | Capture audio locale | #606, #607 |
| Contrôle de l’écran | Actions d’entrée par le portail RemoteDesktop | À cadrer après #608 |
| Démonstrations | Capture des gestes et de l’écran | Dépend de #608 |
| Fenêtre flottante | Fenêtre et raccourci global Linux | #609 |

Le bac à sable YOLO est suivi dans #528.
Les services de #598 comprennent Secret Service, les notifications D-Bus,
les liens profonds et l’ouverture par `xdg-open`.

## Partage d’écran

Le bot voit l’écran le temps d’un bail, pour un fil et une durée. Il n’effectue aucun geste.
Le wrapper lance `ezerbot-linux --screen-helper <ms>`. Son parent doit être le `grokbot-wrapper` voisin.
Sous Wayland, seul le portail `org.freedesktop.portal.Screenshot` fournit une image.
Sans portail, Wayland refuse le bail. Sous X11, le portail passe d’abord, sinon `XGetImage` sur la fenêtre racine.
Le fichier écrit par le portail est retiré dès sa lecture.

Une superposition reste affichée pendant tout le bail, avec **Pause** et **Arrêter**.
La fermer révoque le bail. La réduire ou la couvrir le met en pause, et aucune capture ne part avant son retour à l’écran, même après une reprise.
Sous Wayland, ou sous X11 avec un gestionnaire de composition, rien ne prouve que la superposition reste visible : le bail s’annonce en pause dès le départ et refuse chaque capture.
Seul l’écran entier est partagé. Le partage d’une seule fenêtre est reporté.
Il n’existe pas de touche Échap globale sous Linux. Échap agit quand Ezerbot ou la superposition a le focus.
L’image contient la superposition et la fenêtre d’Ezerbot si elles sont visibles.

Le build de debug ne contient aucune capture réelle et sert `app/native/screen-fixture.png`.
Les tests de capture réelle sont des tests unitaires, avec un Xvfb et un `dbus-daemon` privés.
Ils échouent si `dbus-daemon` manque.

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
Le test échoue si `Xvfb` manque et il ne couvre que les builds de développement.
