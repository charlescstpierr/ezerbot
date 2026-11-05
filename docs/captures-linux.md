# Captures Linux sur un commit exact

`tools/linux-captures.sh <sha>` reconstruit un commit dans un worktree détaché et capture son interface sous Xvfb. Le répertoire de sortie est `artifacts/captures-linux/<sha40>/` dans le checkout qui lance la commande.

Le rendu utilise le vrai wrapper, son interface compilée et un fournisseur factice local. Le manifeste identifie le rendu de chaque vue. Les vues web sont un repli Chromium : elles prouvent l'interface servie sous Linux, pas la coque.

Une étape native complète le pilote. Elle lance la vraie coque GTK de développement avec `EZERBOT_FAKE_SIGNIN=1` dans son propre Xvfb de 1440 × 900 pixels, sous openbox. Elle ouvre une autre fenêtre, envoie le raccourci global et vérifie que le panneau porte `_NET_WM_STATE_ABOVE`. Elle remonte l'autre fenêtre, constate que le panneau reste au-dessus, puis capture tout l'écran avec `import -window root`. Cette vue prouve la coque native, son panneau flottant et son premier plan. Elle ne prouve pas Wayland, dont le compositeur décide du premier plan.

Le pilote injecte `EzerbotShell` avant le chargement, avec la liste de `app/linux/src/local-api.js`. Les vues web montrent donc les textes et les panneaux de Linux. `screen-overlay` lance le vrai `ezerbot-linux --screen-helper` de debug sur le Xvfb des vues web. Sa capture vient de la fenêtre racine. Ce binaire ne contient aucune capture réelle. `screen-overlay-unproven` exige `picom`.

## Prérequis

La commande exige Linux, Git, Bash, Node avec WebSocket, npm, Rust, Chrome ou Chromium, Xvfb et xauth. Elle utilise aussi `flock` et `setsid`. L'étape native exige en plus openbox, xdotool, xprop et ImageMagick (`import`, `display`, `identify`). Un outil absent fait échouer l'étape avec son nom. Sur la box, Node 20 active WebSocket avec `--experimental-websocket`. Rust doit respecter `rust-toolchain.toml`. Sur la box, placez `/home/box/.cargo/bin` avant `/usr/bin` dans `PATH` pour utiliser rustup et Rust 1.99.1.

Le commit demandé doit contenir le pilote de capture et la fixture attendue. Un ancien commit incompatible provoque un refus avant le build. Le checkout qui lance la commande peut avoir des modifications locales. Le worktree détaché ne les reprend pas.

Le verrou propre aux captures sérialise les exécutions de ce checkout. Le cache Cargo dédié est `target/linux-captures/`. Il reste réutilisable après le nettoyage des ressources temporaires.

Le worktree et le profil temporaire utilisent un chemin court sous `/tmp/ezcap.*`. Chrome crée un socket Unix dont le chemin doit rester court.

## Produire les preuves d'une PR

1. Commitez les changements à capturer.
2. Exécutez `tools/linux-captures.sh "$(git rev-parse HEAD)"` depuis le checkout de la PR.
3. Conservez le manifeste et les PNG avant de rejouer la commande.
4. Rejouez la commande sur le même commit. Comparez les identifiants des vues et vérifiez chaque empreinte SHA-256.
5. Joignez le manifeste et des captures à la PR. Sur la box, conservez aussi la preuve dans `/workspace/ezerbot-wt/captures/<PR>-<sha40>/manifest.json`.

Une modification du code après la capture exige de nouvelles captures sur la nouvelle tête. Les preuves restent hors du commit capturé pour éviter que leur ajout change son SHA.

## Isolation et limites

Chaque exécution crée sa propre base, sa fausse session et son profil Chrome. Le fournisseur factice ne contacte aucun modèle distant. Le script réutilise un seul Chrome pour toutes les vues. Il ne lance pas le Chrome personnel et ne charge aucune base existante.

L'outil attend la connexion du wrapper, l'état propre à chaque vue, les polices et le rendu. Un délai maximal provoque un échec. Un délai fixe ne valide pas un écran prêt.

Le pool corrigé de `vendor/tiny_http` donne un fil à chaque connexion keep-alive, ce qui évite les chargements d'assets suspendus observés sur la box. Le pilote signale un document sans focus au module des notifications avant de créer la conversation factice. Sa notification reste ainsi disponible pour la vue dédiée.

Les builds utilisent `CARGO_BUILD_JOBS=3`. Avant chaque build, le script contrôle les systèmes de fichiers du cache Cargo, du worktree temporaire et des captures. Il refuse de continuer si l'un dispose de moins de 25 Gio libres. Sur la box, il affiche aussi `df -BG /workspace`. Les fichiers temporaires et les processus de l'exécution sont nettoyés en cas de réussite ou d'échec.

## À capturer sur le Mac (optionnel)

La fenêtre AppKit, son panneau flottant, ses menus et les services natifs macOS restent des preuves propres au Mac. Le job `compile` continue de vérifier la compilation de la coque macOS.

## Vues attendues

Le registre du pilote fournit ces vues, dans cet ordre. Le viewport est de 1440 × 900 pixels, avec un facteur d'échelle de 1 et le thème sombre.

| Identifiant | État attendu |
| --- | --- |
| `home-dark` | Accueil et compositeur prêts. |
| `conversation-dark` | Conversation synthétique avec sa réponse terminée. |
| `settings-dark` | Dialogue des réglages ouvert. |
| `routines-dark` | Dialogue des routines ouvert. |
| `notifications-dark` | Dialogue ouvert avec la notification de la réponse synthétique. |
| `conversation-tools-dark` | Tour actif après un vrai outil local : seulement la phrase du bot et « Réfléchit », sans aucune trace d'outil. |
| `screen-consent-dark` | Carte d'accord du partage d'écran, ouverte dans la conversation synthétique. |
| `screen-overlay` | Fenêtre racine de Xvfb avec la superposition du bail, au-dessus de Chromium. |
| `screen-overlay-unproven` | La même fenêtre racine sous `picom` : le bail démarre en pause et le bandeau le dit. |
| `settings-microphone-dark` | Réglages ouverts sur « Microphone » : la ligne « Microphone par défaut du système » et les micros factices de la fixture. |
| `floating-panel-native` | Vraie coque native : le panneau flottant ouvert par son raccourci reste au-dessus d'une autre fenêtre remontée. |

## Format du manifeste

`manifest.json` est un objet JSON UTF-8. `schema_version` vaut `1`. `sha` contient les 40 caractères hexadécimaux du commit capturé. `captured_at` donne la date de capture en UTC au format ISO 8601.

`platform` vaut `linux`. `renderer` vaut `chromium-web-fallback` et décrit les vues web. `chrome_version` indique la version réellement interrogée. `viewport` contient `width`, `height` et `device_scale_factor`.

`views` est un tableau ordonné. Chaque entrée contient `id`, `label`, `file`, `renderer`, `sha256`, `width` et `height`. `renderer` vaut `chromium-web-fallback` pour une vue web et `webkitgtk-native-shell` pour la vue native. `file` est le nom relatif du PNG dans le même dossier. `sha256` est l'empreinte hexadécimale SHA-256 des octets du fichier. Une preuve complète contient exactement les vues documentées et tous leurs PNG valides.

Le script vérifie le SHA annoncé par le wrapper compilé. Il ne publie le nouveau manifeste qu'après la réussite de toutes les vues. Une nouvelle exécution peut avoir une autre date de capture. L'acceptation exige les mêmes vues. Elle ne suppose pas que des versions différentes de Chrome produisent les mêmes pixels.
