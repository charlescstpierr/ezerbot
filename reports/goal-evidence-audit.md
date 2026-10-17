# Bilan de l’investigation REA et preuves encore manquantes

L’investigation a réduit les inconnues sur les neuf sujets et conserve leurs preuves. Les exigences de recherche sont remplies ; la connaissance du produit reste partielle. Cette conclusion ne prétend ni récupérer le moteur fermé, ni terminer la parité de notre app.

Les preuves statiques, les déclarations documentaires, le démarrage observé et les tranches originales exécutées dans un test sont distingués. Une hypothèse de backend ne remplace jamais un corps ou une trace.

## Audit de l’objectif

Réduire par des preuves nouvelles les inconnues restantes de Grok Bot : moteur serveur et choix des outils/fin, contexte et compaction, mémoire/oubli, échanges et contexte entre bots, garanties après panne, iPhone/auth/push/synchronisation/outils, démonstration→skill, scheduler et helpers natifs. Réutiliser les analyses sauvegardées, rechercher les artefacts et observations autorisés réellement accessibles, examiner les chemins non couverts, conserver des preuves vérifiables et mettre à jour les rapports. Établir ce qui peut l’être sans inventer les corps serveur absents; identifier précisément toute preuve inaccessible et les limites résiduelles.

Chaque exigence de travail a été confrontée aux fichiers actuels, aux inventaires et aux revues indépendantes :

- Réutiliser les analyses sauvegardées et leur identité.
- Rechercher des artefacts et observations autorisés réellement accessibles.
- Réduire les inconnues sur chacun des neuf sujets par de nouveaux chemins ou preuves.
- Conserver des preuves vérifiables et leur provenance.
- Mettre à jour les rapports et la reprise de session.
- Établir ce qui peut l’être sans inventer les corps serveur absents.
- Identifier précisément les preuves inaccessibles et les limites résiduelles.

Les correspondances détaillées sont conservées dans [goal-evidence-audit.json](../evidence/goal-evidence-audit.json). Les trois revues sont des analyses, pas de nouvelles observations de la cible : [goal-scope-review.json](../evidence/goal-scope-review.json).

## État des neuf questions

### Moteur, outils et fin : Non résolu

La boucle vocale, la réparation de réveil après formulaire et les résultats locaux sont retracés. Un résultat done peut contenir isError:true ; accepted ne prouve pas une reprise du moteur. Sources : [server-orchestration-residual.md](server-orchestration-residual.md), [goal-execution-autonomy.md](goal-execution-autonomy.md), [local-cua-path-0712.md](local-cua-path-0712.md).

Le moteur texte, le choix des outils, la replanification, les budgets globaux et le prédicat de fin ne sont pas récupérés.

Preuve nécessaire : Corps déployé box/Temporal ou trace autorisée demande → run/tour → requêtes modèle → appels/résultats outils → décision terminale, avec compteurs.

### Contexte et compaction : Partiel

Assemblage vocal, rappel filtré, prédescription d’image avec cinq textes humains et contrat de résumé avec bornes de troncature/reprise sont établis. Sources : [server-context-residual.md](server-context-residual.md), [goal-context-memory-learning.md](goal-context-memory-learning.md), [compaction-runtimes.md](compaction-runtimes.md).

Le prompt texte complet, sa priorité, le seuil effectif, le résumé réellement produit et la continuité après erreur restent inconnus.

Preuve nécessaire : Assembleur appelé et producteur/consommateur du résumé, ou requêtes modèle avant/après corrélées : modèle, tokens, configuration effective, bornes et mémoire/version.

### Mémoire, correction et oubli : Partiel

Sélection, promotion, faits traités privés inclus, reçu optimiste et retry du widget sont retracés. Une promotion réussie peut subsister après l’échec de son reçu. Sources : [memory-widget-consumer.md](memory-widget-consumer.md), [server-context-residual.md](server-context-residual.md).

Extraction, correction durable, oubli, propagation aux résumés/caches et injection dans la requête modèle ne sont pas démontrés.

Preuve nécessaire : Implémentation du service de mémoire et trace corrélée d’une correction/retrait : versions avant/après, propagation et relecture. Le nom d’une opération d’oubli n’est pas établi.

### Échanges et contexte entre bots : Partiel

Mentions riches, membres, nouvelles rooms, identités, replies, transport et retours vocaux sont suivis. Une mention conserve le fil courant ; le changement des membres est une opération distincte. Sources : [multibot-membership-paths.md](multibot-membership-paths.md), [multibot-turn-paths.md](multibot-turn-paths.md), [interbot-client-paths.md](interbot-client-paths.md).

L’émetteur serveur, l’arbitrage du prochain bot, l’anti-boucle, l’héritage du contexte et la livraison durable restent inconnus.

Preuve nécessaire : Traitement serveur de SendGrokBotAgentMessage et des tours de room, constructeur de requête modèle du destinataire, trace expéditeur → réception → run avec IDs et contexte choisi.

### Pannes et effets incertains : Partiel

Caches B4 volatils, remplacement du worker sans replay automatique, UUID/retry et chemin natif jusqu’à osascript sont établis. Quatre cas exécutent des tranches originales avec frontières fictives. Sources : [messages-worker-recovery.md](messages-worker-recovery.md), [native-messages-recovery.md](native-messages-recovery.md), [messages-worker-slice-replay.md](messages-worker-slice-replay.md).

La livraison réelle, l’unicité durable d’effet et la transaction effet/résultat ne sont pas prouvées. Les compteurs de la sonde ne sont pas des doublons Messages observés.

Preuve nécessaire : Scénario de panne autorisé sur environnement de test, avec effet et reçu durable corrélés avant/après redémarrage ; pour les outils distants, journal d’effets/outbox et contrats transactionnels.

### iPhone, auth, push, synchronisation et outils : Partiel

Identité iOS et domaines officiels, login Cursor du Mac, roster, flux transcript Mac et interfaces communes sont établis. Le champ mobileLocalTools examiné est annoncé par le daemon Mac. Sources : [distribution-mobile-discovery.md](distribution-mobile-discovery.md), [interbot-mobile-protocol.md](interbot-mobile-protocol.md), [local-cua-path-0712.md](local-cua-path-0712.md).

Entitlements et login iOS effectifs, APNs, fond, synchronisation iOS, outils et décision serveur de routage ne sont pas récupérés.

Preuve nécessaire : App/IPA iOS autorisée analysable et versionnée, ou sources ; service push/routage ou captures passives corrélées des deux clients. Aucun paquet ni outil iPhone disponible dans l’accès vérifié.

### Démonstration vers skill : Partiel

Start, save, discard, statut et erreurs Teach sont suivis. Le chemin séparé d’import texte crée des skills avec résultats partiels, sans consommer d’enregistrement Teach. Sources : [goal-context-memory-learning.md](goal-context-memory-learning.md), [server-orchestration-residual.md](server-orchestration-residual.md).

La transformation média → procédure → brouillon/skill, sa validation, ses versions et sa sélection restent inconnues.

Preuve nécessaire : Enregistrement Teach autorisé, job d’extraction et résultat corrélé recording-ID → skill-ID/version, puis preuve de sauvegarde et de sélection.

### Scheduler et runs de routine : Partiel

Configuration et relectures sont retracées ; 26 tranches originales produisent les 16 résultats de calcul candidat attendus. Les modifications de routine sont séparées des exécutions. Sources : [cron-detail.md](cron-detail.md), [cron-candidate-replay.md](cron-candidate-replay.md), [routine-run-conversation.md](routine-run-conversation.md).

Départs réels, rattrapage, déduplication, concurrence et jointure run → session → résultat ne sont pas établis. Le seul appel direct nommé au calcul retrouvé décrit un intervalle.

Preuve nécessaire : Implémentation du scheduler ou occurrences enregistrées avec zone effective, clé d’occurrence, départ, run ID, session et entrées de transcript corrélés.

### Helpers natifs et effets macOS : Partiel

Admission du peer, signatures, leases, révocation, événements compensatoires et façade MCP/socket sont suivis aux octets. Neuf corps arm64 concernent .70 ; les nouvelles analyses .71 sont x64. Sources : [native-input-residual.md](native-input-residual.md), [native-arm64-residual.md](native-arm64-residual.md), [native-0712-mcp.md](native-0712-mcp.md), [local-cua-consent-0712.md](local-cua-consent-0712.md).

Livraison effective CGEvent/AX, courses, réussite de libération et équivalence arm64 .71 restent non prouvées. Aucun inventaire exhaustif de tous les symboles n’est annoncé.

Preuve nécessaire : Observation contrôlée de la version concernée, événements et réceptions corrélés, révocation/arrêt/reprise, puis corps ciblés supplémentaires si une divergence apparaît.

## Accès réellement vérifié

La vérification datée 2026-10-17T05:33:31.312937+00:00 ne trouve aucun fichier IPA/APK/HAR/mitm/flow dans les deux racines sélectionnées. Les six outils iPhone cherchés ne sont pas disponibles. La requête USB ne trouve aucun produit nommé iPhone. Ces résultats sont bornés aux chemins, suffixes, outils et noms interrogés ; ils ne décrivent pas tous les dossiers ou appareils possibles.

Aucun numéro de série, profil, jeton ou compte n’a été lu. L’état de connexion actuel du compte reste inconnu. Le corpus conserve un démarrage original sur écran de connexion, sans scénario authentifié observé. Sources : [goal-access-audit.json](../evidence/goal-access-audit.json), [original-runtime-verification.json](../evidence/original-runtime-verification.json).

Le contrôle d’accès se rejoue explicitement avec `python3 tools/inspect-goal-access.py --capture`. Une nouvelle capture change les preuves et exige un nouvel audit ; la vérification ci-dessous ne lance pas cette commande.

## Portée de la conclusion

- L’analyse ne prétend pas couvrir toutes les fonctions du paquet ou toutes les versions.
- Les six contradictions d’intégrité ASAR de chaque distribution restent conservées. Le helper Computer Use hors ASAR n’en fait pas partie.
- Le sous-ensemble arm64 .70 ne prouve pas l’équivalence arm64 .71.
- Les compteurs fictifs de Messages et les instants candidats cron ne sont pas des effets externes observés.
- La signature, le retour d’un outil et un état de carte ne prouvent pas la réussite métier ou la livraison.
- Une observation authentifiée peut préciser un scénario sans révéler le prompt ou l’algorithme privé.
- Les artefacts externes listés ci-dessus permettraient une nouvelle investigation. Ils ne sont pas remplacés par des suppositions.

## Vérification

`python3 tools/verify-goal-audit.py` contrôle les identités, les empreintes des fichiers, les neuf rubriques, les sept correspondances et les classes de preuves. Il ne relance ni cible, ni tranches JavaScript, ni réseau, ni inspection de téléphone. Ce contrôle garantit la cohérence du dossier ; il ne vérifie pas les mécanismes serveur encore inconnus.
