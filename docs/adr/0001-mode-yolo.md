# 0001 — Mode YOLO activé par défaut, par recouvrement

## Contexte

Les capacités puissantes (terminal, fichiers, écran, connecteurs) étaient désactivées par bot, et chaque action protégée affichait une carte d'approbation. Le propriétaire de cette application personnelle veut l'inverse, comme le mode yolo de Codex : tout ouvert et rien qui demande, tout en gardant la trace de ce qui a été fait.

## Décision

Un interrupteur global « Mode YOLO (tous les droits) », **activé par défaut** pour les bases neuves et existantes, plus une surcharge par bot (suivre le global, toujours agir, toujours demander).

- Le mode **recouvre** les réglages sans les écraser : `local::enabled`, `screen::enabled` et `mcp::enabled` répondent vrai tant qu'il s'applique, et les tables de réglages ne sont pas touchées. Repasser à « Demander avant d'agir » rend donc les réglages d'avant, sans migration inverse. La migration ne fait qu'insérer la ligne `yolo` une seule fois (`INSERT OR IGNORE`) ; un choix « ask » n'est jamais rejoué.
- Le mode se branche sur la porte d'approbation existante (`check_tool_approval`) avec une nouvelle origine d'autorisation `AuthorizedBy::Yolo { source }`. L'admission, l'entrée du journal d'activité (`wrapper_yolo_log`) et la mention portée par le résultat d'outil partent dans la même transaction. Le fil rend cette origine par la carte « Fait automatiquement ».

## Garde-fous qui survivent au mode

- Les données d'Ezerbot (dossier de la base, coffre, jetons, trousseau) sont **refusées**, pas mises en carte : par chemin réel et inode pour les fichiers, par ce que nomme la commande pour le terminal (qui n'est pas un sandbox). Le refus est revérifié à l'exécution.
- Un « Toujours refuser » écrit par l'utilisateur gagne encore.
- Le contrôle de l'écran exige toujours la session démarrée par l'utilisateur ; Échap et Arrêter coupent le bot comme avant ; macOS demande lui-même ses permissions.
- Restent des cartes, parce qu'elles décident ou font relire plutôt qu'elles n'autorisent un outil : les brouillons d'envoi (courriel, message, commentaire, issue), `propose_skill`, le démarrage et le retrait d'un connecteur, le consentement Recall, la publication des webhooks et l'agent de routines.

## Conséquence

Une routine ou un sous-agent d'un bot en YOLO agit sans personne devant l'écran. C'est voulu ; le journal d'activité et la carte du fil sont la trace.
