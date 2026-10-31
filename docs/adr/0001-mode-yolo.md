# 0001 — Mode YOLO activé par défaut, par recouvrement

## Contexte

Les capacités puissantes (terminal, fichiers, écran, connecteurs) étaient désactivées par bot, et chaque action protégée affichait une carte d'approbation. Le propriétaire de cette application personnelle veut l'inverse, comme le mode yolo de Codex : tout ouvert et rien qui demande, tout en gardant la trace de ce qui a été fait.

## Décision

Un interrupteur global « Mode YOLO (tous les droits) », **activé par défaut** pour les bases neuves et existantes, plus une surcharge par bot (suivre le global, toujours agir, toujours demander).

- Le mode **recouvre** les réglages sans les écraser : `local::enabled` et `screen::enabled` répondent vrai tant qu'il s'applique, et les tables de réglages ne sont pas touchées. Les connecteurs font exception : `mcp::enabled` ne lit jamais le mode, car décocher « Activer les outils pour » un bot est une coupure que YOLO ne rouvre pas ; le mode ne lève que la carte des outils des connecteurs activés. Repasser à « Demander avant d'agir » rend donc les réglages d'avant, sans migration inverse. La migration ne fait qu'insérer la ligne `yolo` une seule fois (`INSERT OR IGNORE`) ; un choix « ask » n'est jamais rejoué.
- Le mode se branche sur la porte d'approbation existante (`check_tool_approval`) avec une nouvelle origine d'autorisation `AuthorizedBy::Yolo { source }`. L'admission, l'entrée du journal d'activité (`wrapper_yolo_log`) et la mention portée par le résultat d'outil partent dans la même transaction. Le fil rend cette origine par la carte « Fait automatiquement ».

## Garde-fous qui survivent au mode

- Les données d'Ezerbot (dossier de la base, coffre, jetons, trousseau) sont **refusées**, pas mises en carte : par chemin réel, inode et racine protégée (dossier de la base, installation habituelle `~/Library/Application Support/Grokbot`, trousseaux) pour les fichiers, par ce que nomme la commande pour le terminal, et, sur macOS, par un `sandbox-exec` qui interdit au shell et à ses enfants le dossier de la base, le dossier de support d'Ezerbot et le trousseau (fichiers et services `securityd`), pour ce qu'une commande atteint sans le nommer. Le refus est revérifié à l'exécution.
- L'API locale du wrapper exige un secret aléatoire neuf à chaque lancement. La coque le transmet par un pipe privé et authentifie ses appels par un en-tête, sans stockage sur disque. Seuls les GET de la liste fermée des assets statiques sont publics après contrôle Host. L'accès iPhone conserve son cookie d'appareil et les webhooks leur Bearer sur des listeners distincts. Une commande de terminal qui nomme un des ports de contrôle reste refusée. Sur macOS, le `sandbox-exec` interdit encore au shell toute connexion vers `localhost`, donc vers tout service local de la machine.
- Sous Linux, bubblewrap (`/usr/bin/bwrap`) joue le rôle de `sandbox-exec` : il recouvre, pour le shell et ses enfants, les mêmes racines protégées que les outils fichiers, plus le trousseau Linux (`~/.local/share/keyrings`, `~/.local/share/kwalletd`, le bus de session et `keyring` sous `$XDG_RUNTIME_DIR`). Si le bac à sable ne démarre pas (bubblewrap absent, espaces de noms utilisateur interdits), le mode reste éteint avec un message clair : les outils demandent leur carte, la base garde son réglage et le mode revient de lui-même une fois bubblewrap installé et Ezerbot relancé. Landlock est écarté : c'est une liste d'autorisation, il interdit de créer un fichier dans tout ancêtre des données et ne sait pas refuser un sous-dossier d'un dossier ouvert. Limite : sous Linux, `localhost` reste joignable depuis le shell ; seul le secret de lancement protège l'API.
- Un « Toujours refuser » écrit par l'utilisateur gagne encore.
- Le contrôle de l'écran exige toujours la session démarrée par l'utilisateur, dont le consentement suit le mode effectif du bot : un accord coché sous l'autre mode n'est pas envoyé et le dialogue se redessine ; Échap et Arrêter coupent le bot comme avant ; macOS demande lui-même ses permissions.
- Restent des cartes, parce qu'elles décident ou font relire plutôt qu'elles n'autorisent un outil : les brouillons d'envoi (courriel, message, commentaire, issue), `propose_skill`, le démarrage et le retrait d'un connecteur, le consentement Recall, la publication des webhooks et l'agent de routines.

## Conséquence

Une routine ou un sous-agent d'un bot en YOLO agit sans personne devant l'écran. C'est voulu ; le journal d'activité et la carte du fil sont la trace.
