# Ouvrir Ezerbot sous Linux

Installez Rust avec rustup. Sur Debian ou Ubuntu, installez les bibliothèques système.

```sh
sudo apt install build-essential pkg-config libgtk-3-dev libwebkit2gtk-4.1-dev xdg-utils
tools/build-app-linux.sh
"app/build/linux/Ouvrir Ezerbot.sh"
```

Le script produit les deux exécutables, une icône, un lanceur et l’installateur dans `app/build/linux`.
Le lanceur utilise les exécutables de son dossier et reste utilisable après déplacement du bundle.
Pour installer l’application et créer un raccourci dans le menu, suivez la [notice d’installation](../../docs/install-linux.md).
Le script respecte `CARGO` et le dossier de compilation effectif de Cargo.
`EZERBOT_BUILD_DEBUG=1` produit une version de développement.

Les conversations et les notations restent dans le dossier de données XDG d’Ezerbot.
L’option `--data-dir /chemin/isolé` permet un lancement avec des données séparées.
Le menu **Quitter Ezerbot**, Ctrl+Q et la fermeture de la fenêtre arrêtent aussi le wrapper.
Relancer Ezerbot sur le même dossier de données montre la fenêtre déjà ouverte.
Le script enregistre `ezerbot-linux` pour les liens `ezerbot://`. Une coque déjà ouverte reçoit le lien.
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
| Dictée | Reconnaissance vocale native | #607 |
| Mémos vocaux | Reconnaissance vocale, branchement du flux du micro | #607 |
| Appels et mode voix | Branchement du flux du micro et lecture audio native | #607 |
| Enregistrement local de réunion | Branchement du flux du micro et transcription | #607 |
| Partage d’écran sous Wayland ou avec un compositeur | Portail ScreenCast et son indicateur système | #632 |
| Contrôle de l’écran | Actions d’entrée par le portail RemoteDesktop | À cadrer après #608 |
| Démonstrations | Capture des gestes et de l’écran | Dépend de #608 |

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

## Partage d’écran

Le bot voit l’écran le temps d’un bail, pour un fil et une durée. Il n’effectue aucun geste.
Le partage fonctionne seulement sous X11 sans compositeur. Sous X11 avec un compositeur, il reste en pause et aucune capture ne part. Sous Wayland, le bail est refusé. La suite, par le portail du bureau, est dans #632.
Le wrapper lance `ezerbot-linux --screen-helper <ms>`. Son parent doit être le `grokbot-wrapper` voisin.
L’image vient de `XGetImage` sur la fenêtre racine. Aucun fichier n’est écrit.
Sous Wayland, la racine de XWayland ne montre que les clients X : le bail est refusé plutôt que d’envoyer un faux bureau.

Une superposition reste affichée pendant tout le bail, avec **Pause** et **Arrêter**.
La fermer révoque le bail. La réduire ou la couvrir le met en pause. Réduite, elle revient seule. La reprise est refusée tant qu’elle n’est pas visible à l’écran.
Sous X11 avec un compositeur, rien ne prouve que la superposition reste visible : le bail s’annonce en pause dès le départ, affiche « Partage en pause : bandeau non garanti visible », et refuse chaque capture comme chaque reprise.
Seul l’écran entier est partagé. Le partage d’une seule fenêtre est reporté.
Il n’existe pas de touche Échap globale sous Linux. Échap agit quand Ezerbot ou la superposition a le focus.
L’image contient la superposition et la fenêtre d’Ezerbot si elles sont visibles.

Le build de debug ne contient aucune capture réelle et sert `app/native/screen-fixture.png`.
Les tests de capture réelle sont des tests unitaires, avec un Xvfb privé.

## Preuve sans compte réel

Construisez avec `EZERBOT_BUILD_DEBUG=1`, puis lancez avec `EZERBOT_FAKE_SIGNIN=1`
et un `--data-dir` temporaire. Le faux fournisseur local répond `OK`.
Il existe uniquement dans le wrapper de développement.
Les preuves de la coque doivent utiliser sa vraie fenêtre sous Xvfb.
Sous Xvfb, utilisez `WEBKIT_DISABLE_DMABUF_RENDERER=1` pour permettre le rendu
de WebKit sans export de buffers graphiques. Le lanceur ne force pas cette option.

`cargo test -p ezerbot-linux` construit le wrapper puis ouvre la vraie fenêtre dans son propre Xvfb.
`tests/window.rs` constate l’affichage, fait un échange avec le faux fournisseur et clique des liens.
Il vérifie aussi la vraie connexion jusqu’à `xdg-open`, la relance de l’application déjà ouverte
et l’arrivée d’un lien dans la fenêtre.
Les tests du panneau lancent aussi `openbox`.
Ils envoient le raccourci, vérifient `_NET_WM_STATE_ABOVE` et prennent la touche avec un second client X.
Le test échoue si `Xvfb` ou `openbox` manque. Aucun test n'est sauté, y compris sur GitHub Actions.
Aucun test ne lance le portail GlobalShortcuts. Wayland est traité dans #635.
Ils ne couvrent que les builds de développement.
