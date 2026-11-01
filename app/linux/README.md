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
| Dictée | Reconnaissance vocale native | #607 |
| Mémos vocaux | Reconnaissance vocale, branchement du flux du micro | #607 |
| Appels et mode voix | Branchement du flux du micro et lecture audio native | #607 |
| Enregistrement local de réunion | Branchement du flux du micro et transcription | #607 |
| Capture de l’écran | Permissions et services du bureau | #608 |
| Contrôle de l’écran | Actions d’entrée par le portail RemoteDesktop | Optionnel dans #608, à cadrer |
| Démonstrations | Capture des gestes et de l’écran | Dépend de #608 |
| Fenêtre flottante | Fenêtre et raccourci global Linux | #609 |

Le choix du micro est porté (#606) : **Paramètres › Micro** liste les entrées, le défaut du système en premier,
mémorise le choix et annonce le repli sur le défaut quand le micro choisi a disparu.
Le wrapper pilote le serveur audio avec ses outils en ligne de commande, sans bibliothèque liée :
`pipewire-bin` (`pw-dump`, `pw-record`) ou `pulseaudio-utils` (`pactl`, `parec`).
PipeWire passe en premier, puis PulseAudio, qui répond aussi via `pipewire-pulse`.
Sans l’un des deux, la liste annonce que le serveur audio est indisponible et le choix mémorisé est conservé.
Le flux est du PCM 16 kHz mono 16 bits, comme sous macOS. Il n’a pas encore de consommateur (#607).
Les versions de développement gardent les micros factices et n’atteignent jamais le matériel.

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
Le test échoue si `Xvfb` manque et il ne couvre que les builds de développement.
